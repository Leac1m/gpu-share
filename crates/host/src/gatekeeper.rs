//! The gatekeeper (`P2-T5`, spec §6.8): the only part of a host reachable from the network.
//!
//! * HTTPS (rustls) with the install's own certificate; clients are asked for theirs.
//! * Routes come from [`protocol::gatekeeper::ENDPOINTS`] and nothing else. Every other path,
//!   and every other method on a known path, is a plain 404.
//! * `Paired` endpoints need a client certificate whose fingerprint is in the paired list *and*
//!   that device's token as `Authorization: Bearer`.
//! * [`Gatekeeper::start_owner`] runs a second listener on `127.0.0.1` for the owner's own use
//!   of the GPU (`P9-T1`): the same table minus pairing and discovery, open only to this
//!   install's own certificate with a token that lives in memory.
//! * Chat requests wait in the [`crate::core`] line; while they wait, the stream carries
//!   [`QueueEvent`]s. The engine's work is cancelled when the client disconnects, presses Stop,
//!   or the host pauses (after [`PAUSE_GRACE`]).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{on, MethodFilter};
use axum::{Extension, Json, Router};
use futures_util::StreamExt;
use hyper_util::rt::TokioIo;
use protocol::api::{ApiError, ChatRequest, ErrorKind, ModelInfo, ModelList, QueueEvent, QUEUE_EVENT};
use protocol::gatekeeper::{Access, Method, ENDPOINTS};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tower::ServiceExt;

use crate::core::{HostCore, JobPhase, JoinRefused, Ticket, Turn, OWNER, PAUSE_GRACE};
use crate::store::{Device, Usage};

/// Requests larger than this are refused before parsing.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Request headers must arrive within this (slow-header attacks).
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// Open connections at once. A home has a handful of friends; anything beyond this is refused.
pub const MAX_CONNECTIONS: usize = 64;

/// Who is on the other end of a connection.
#[derive(Debug, Clone)]
pub struct Peer {
    pub fingerprint: Option<String>,
    pub addr: SocketAddr,
}

/// A running gatekeeper; dropping it (or calling [`Gatekeeper::stop`]) closes the port and
/// every open connection.
pub struct Gatekeeper {
    shutdown: watch::Sender<bool>,
    pub addr: SocketAddr,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Gatekeeper {
    pub async fn start(core: Arc<HostCore>, bind: SocketAddr) -> std::io::Result<Self> {
        let app = router(core.clone());
        Self::serve(core, bind, app).await
    }

    /// The owner's own listener (`P9-T1`), on a free port of `127.0.0.1`. It runs whether
    /// sharing is on or off; `token` is handed to this app's own Use mode and never stored.
    pub async fn start_owner(core: Arc<HostCore>, token: String) -> std::io::Result<Self> {
        let app = owner_router(core.clone(), token);
        Self::serve(core, SocketAddr::from(([127, 0, 0, 1], 0)), app).await
    }

    async fn serve(core: Arc<HostCore>, bind: SocketAddr, app: Router) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let tls = netcore::tls::server_config(&core.identity).map_err(std::io::Error::other)?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let (shutdown, mut stop) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut conns = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = stop.changed() => break,
                    accepted = listener.accept() => {
                        let Ok((tcp, addr)) = accepted else { continue };
                        if conns.len() >= MAX_CONNECTIONS {
                            drop(tcp);
                            continue;
                        }
                        let acceptor = acceptor.clone();
                        let app = app.clone();
                        conns.spawn(async move {
                            let Ok(Ok(tls)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await else {
                                return;
                            };
                            let peer = Peer {
                                fingerprint: netcore::tls::peer_fingerprint(tls.get_ref().1.peer_certificates()),
                                addr,
                            };
                            let svc = hyper::service::service_fn(move |mut req: Request<hyper::body::Incoming>| {
                                req.extensions_mut().insert(peer.clone());
                                app.clone().oneshot(req.map(Body::new))
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .timer(hyper_util::rt::TokioTimer::new())
                                .header_read_timeout(HEADER_TIMEOUT)
                                .serve_connection(TokioIo::new(tls), svc)
                                .await;
                        });
                    }
                    Some(_) = conns.join_next(), if !conns.is_empty() => {}
                }
            }
            conns.abort_all();
        });
        Ok(Self { shutdown, addr, task: Some(task) })
    }

    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for Gatekeeper {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// The router, built from the endpoint table so it can't quietly grow (spec §7).
pub fn router(core: Arc<HostCore>) -> Router {
    let mut r = Router::new();
    for ep in ENDPOINTS {
        let filter = match ep.method {
            Method::Get => MethodFilter::GET,
            Method::Post => MethodFilter::POST,
        };
        let route = match ep.path {
            "/api/info" => on(filter, info),
            "/api/register" => on(filter, register),
            "/api/pair" => on(filter, crate::pairing::pair),
            "/v1/models" => on(filter, models),
            "/v1/chat/completions" => on(filter, chat),
            other => panic!("no handler for {other}: add one, after updating spec §6.8"),
        }
        .fallback(not_found);
        let route = match ep.access {
            Access::Anyone => route,
            Access::Paired => route.route_layer(middleware::from_fn_with_state(core.clone(), require_paired)),
        };
        r = r.route(ep.path, route);
    }
    r.fallback(not_found).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)).with_state(core)
}

/// The owner's listener: the same endpoint table, but pairing and discovery aren't offered, and
/// `Paired` endpoints accept only this install's own certificate with the in-memory token.
pub fn owner_router(core: Arc<HostCore>, token: String) -> Router {
    let auth = OwnerAuth { core: core.clone(), token: Arc::new(token) };
    let mut r = Router::new();
    for ep in ENDPOINTS {
        let filter = match ep.method {
            Method::Get => MethodFilter::GET,
            Method::Post => MethodFilter::POST,
        };
        let route = match ep.path {
            "/api/info" => on(filter, info),
            "/v1/models" => on(filter, models),
            "/v1/chat/completions" => on(filter, chat),
            _ => continue, // pairing and discovery are for other computers
        }
        .fallback(not_found);
        let route = match ep.access {
            Access::Anyone => route,
            Access::Paired => route.route_layer(middleware::from_fn_with_state(auth.clone(), require_owner)),
        };
        r = r.route(ep.path, route);
    }
    r.fallback(not_found).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)).with_state(core)
}

#[derive(Clone)]
struct OwnerAuth {
    core: Arc<HostCore>,
    token: Arc<String>,
}

/// Owner endpoints: a loopback connection, this install's own certificate (its private key
/// never leaves the data folder), and the token this run of the app made.
async fn require_owner(State(auth): State<OwnerAuth>, mut req: Request, next: Next) -> Response {
    let unauthorized = || api_error(StatusCode::UNAUTHORIZED, ErrorKind::Unauthorized, "Only this computer's own GPU Share can use this.");
    let Some(peer) = req.extensions().get::<Peer>().cloned() else {
        return unauthorized();
    };
    let own_cert = peer.fingerprint.as_deref() == Some(auth.core.identity.fingerprint());
    let token_ok = bearer(req.headers()).is_some_and(|t| constant_time_eq(t.as_bytes(), auth.token.as_bytes()));
    if !(peer.addr.ip().is_loopback() && own_cert && token_ok) {
        return unauthorized();
    }
    req.extensions_mut().insert(Device { id: OWNER.into(), name: "You".into() });
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

pub fn api_error(status: StatusCode, kind: ErrorKind, msg: impl Into<String>) -> Response {
    (status, Json(ApiError::new(kind, msg))).into_response()
}

/// `Paired` endpoints: the client certificate must be a paired device's, and the bearer token
/// must be that device's token.
async fn require_paired(State(core): State<Arc<HostCore>>, mut req: Request, next: Next) -> Response {
    let unauthorized = || api_error(StatusCode::UNAUTHORIZED, ErrorKind::Unauthorized, "This computer isn't paired with this host.");
    let Some(fp) = req.extensions().get::<Peer>().and_then(|p| p.fingerprint.clone()) else {
        return unauthorized();
    };
    let Some(token) = bearer(req.headers()) else {
        return unauthorized();
    };
    match core.store.authorize(&fp, &token) {
        Ok(Some(device)) => {
            req.extensions_mut().insert(device);
            next.run(req).await
        }
        _ => unauthorized(),
    }
}

fn bearer(h: &HeaderMap) -> Option<String> {
    h.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(|t| t.trim().to_string())
}

async fn info(State(core): State<Arc<HostCore>>) -> Response {
    Json(core.announcement(true)).into_response()
}

/// Discovery reply (spec §6.6): the caller says who it is, the host answers with who it is.
/// Nothing is stored.
async fn register(State(core): State<Arc<HostCore>>, body: Bytes) -> Response {
    if serde_json::from_slice::<protocol::discovery::Announcement>(&body).is_err() {
        return api_error(StatusCode::BAD_REQUEST, ErrorKind::InvalidRequest, "Expected an announcement.");
    }
    Json(core.announcement(false)).into_response()
}

async fn models(State(core): State<Arc<HostCore>>, Extension(device): Extension<Device>) -> Response {
    let list = if device.id == OWNER { core.owner_models() } else { core.offered() };
    let data = list
        .into_iter()
        .map(|e| ModelInfo { id: e.id, object: "model".into(), owned_by: core.cfg.name.clone(), name: e.name })
        .collect();
    Json(ModelList::new(data)).into_response()
}

// ------------------------------------------------------------------------- chat

/// What the turn task hands back to the response.
enum Out {
    Queue(QueueEvent),
    Chunk(Value),
}

async fn chat(State(core): State<Arc<HostCore>>, Extension(device): Extension<Device>, body: Bytes) -> Response {
    let req: ChatRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, ErrorKind::InvalidRequest, format!("Bad request: {e}")),
    };
    let owner = device.id == OWNER;
    let spec = if owner { core.owner_model_spec(&req.model) } else { core.model_spec(&req.model) };
    let Some(spec) = spec else {
        return api_error(StatusCode::BAD_REQUEST, ErrorKind::UnknownModel, "This host doesn't offer that model.");
    };
    let limits = if owner { core.owner_limits() } else { core.limits() };
    let engine_body = match crate::clamp::engine_body(&req, limits, &spec.id) {
        Ok(b) => b,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, ErrorKind::InvalidRequest, e.to_string()),
    };
    if owner && !core.engine_ready() {
        // on Linux the engine is fetched on first use: the owner's own question counts
        core.want_engine();
        return api_error(StatusCode::SERVICE_UNAVAILABLE, ErrorKind::Paused, "The AI engine is being set up (a one-time download). Try again in a minute");
    }
    let joined = if owner { core.join_owner() } else { core.join(&device.id) };
    let ticket = match joined {
        Ok(t) => t,
        Err(JoinRefused::Closed(reason)) => return api_error(StatusCode::SERVICE_UNAVAILABLE, ErrorKind::Paused, reason),
        Err(JoinRefused::AlreadyQueued) => {
            return api_error(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorKind::AlreadyQueued,
                "You already have a question waiting. Wait for it, or cancel it first.",
            )
        }
    };
    let (tx, rx) = mpsc::channel::<Out>(64);
    tokio::spawn(run_turn(core.clone(), ticket, spec, engine_body, tx));

    if req.stream.unwrap_or(false) {
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            let frame = match rx.recv().await? {
                Out::Queue(ev) => format!("event: {QUEUE_EVENT}\ndata: {}\n\n", serde_json::to_string(&ev).unwrap()),
                Out::Chunk(v) => format!("data: {v}\n\n"),
            };
            Some((Ok::<_, std::convert::Infallible>(frame), rx))
        })
        .chain(futures_util::stream::once(async { Ok("data: [DONE]\n\n".to_string()) }));
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(stream))
            .unwrap()
    } else {
        collect_reply(rx, &req.model).await
    }
}

/// Non-streaming clients get one OpenAI completion object once the turn is over.
async fn collect_reply(mut rx: mpsc::Receiver<Out>, model: &str) -> Response {
    let mut text = String::new();
    let mut usage = Value::Null;
    let mut started = false;
    while let Some(out) = rx.recv().await {
        match out {
            Out::Queue(QueueEvent::Started) => started = true,
            Out::Queue(QueueEvent::Stopped { reason }) if !started => {
                return api_error(StatusCode::SERVICE_UNAVAILABLE, ErrorKind::Paused, reason)
            }
            Out::Queue(_) => {}
            Out::Chunk(c) => {
                if let Some(t) = c["choices"][0]["delta"]["content"].as_str() {
                    text.push_str(t);
                }
                if !c["usage"].is_null() {
                    usage = c["usage"].clone();
                }
            }
        }
    }
    Json(json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
        "usage": usage,
    }))
    .into_response()
}

/// One request's whole life: wait in line, get the engine ready, stream, count usage.
/// Returning (or the client going away, which closes `tx`) drops the ticket, which frees the
/// card for the next request.
async fn run_turn(core: Arc<HostCore>, mut ticket: Ticket, spec: crate::engine::ModelSpec, body: Value, tx: mpsc::Sender<Out>) {
    // 1. wait our turn
    let mut told = 0;
    loop {
        let turn = ticket.rx.borrow_and_update().clone();
        match turn {
            Turn::Go => break,
            Turn::Stopped(reason) => {
                let _ = tx.send(Out::Queue(QueueEvent::Stopped { reason })).await;
                return;
            }
            Turn::Waiting(pos) if pos != told && pos > 0 => {
                told = pos;
                if tx.send(Out::Queue(QueueEvent::Queued { position: pos })).await.is_err() {
                    return;
                }
            }
            Turn::Waiting(_) => {}
        }
        tokio::select! {
            r = ticket.rx.changed() => if r.is_err() { return },
            _ = tx.closed() => return,
        }
    }

    // 2. the right model on the card (stopping another model's process if needed), if it fits
    if core.supervisor.status().loaded.as_deref() != Some(spec.id.as_str()) {
        if let Some(short) = core.vram_shortfall(&spec) {
            log::info!("not loading {}: {short:.1} GB short", spec.id);
            let (reason, toast) = if core.cfg.card.is_some() {
                ("There isn't enough free graphics card memory on the host right now", ("Not enough graphics card memory", "Something else is using the card, so the model wasn't loaded."))
            } else {
                ("There isn't enough free memory on the host right now", ("Not enough memory", "Other programs are using the memory, so the model wasn't loaded."))
            };
            core.toast(toast.0, toast.1);
            let reason = reason.to_string();
            let _ = tx.send(Out::Queue(QueueEvent::Stopped { reason })).await;
            return;
        }
    }
    ticket.set_phase(JobPhase::Loading);
    let (ptx, mut prx) = watch::channel(0u8);
    let handle = {
        let ensure = core.supervisor.ensure(&spec, |p| {
            let _ = ptx.send(p);
        });
        tokio::pin!(ensure);
        let mut last_pct = None;
        loop {
            tokio::select! {
                h = &mut ensure => break h,
                _ = prx.changed() => {
                    let pct = *prx.borrow_and_update();
                    if last_pct != Some(pct) {
                        last_pct = Some(pct);
                        let _ = tx.send(Out::Queue(QueueEvent::Loading { model: spec.id.clone(), pct })).await;
                    }
                }
                r = ticket.rx.changed() => {
                    if r.is_err() { return }
                    let turn = ticket.rx.borrow_and_update().clone();
                    if let Turn::Stopped(reason) = turn {
                        let _ = tx.send(Out::Queue(QueueEvent::Stopped { reason })).await;
                        return;
                    }
                }
                _ = tx.closed() => return,
            }
        }
    };
    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            log::warn!("engine start failed: {e}");
            core.toast("The model couldn't start", e.to_string());
            let reason = "The model couldn't be started on the host".to_string();
            let _ = tx.send(Out::Queue(QueueEvent::Stopped { reason })).await;
            return;
        }
    };

    // 3. stream from the engine
    ticket.set_phase(JobPhase::Streaming);
    let _ = tx.send(Out::Queue(QueueEvent::Started)).await;
    let started = Instant::now();
    let mut usage = Usage { requests: 1, ..Default::default() };
    core.set_loaded_for_owner(ticket.device == OWNER);
    let outcome = stream_engine(&core, &mut ticket, &handle, &spec.id, body, &tx, &mut usage).await;
    usage.gpu_ms = started.elapsed().as_millis() as u64;
    if ticket.device != OWNER {
        core.record_usage(&ticket.device, usage); // counts are per friend; the owner's own use isn't one
    }
    if let Err(reason) = outcome {
        let _ = tx.send(Out::Queue(QueueEvent::Stopped { reason })).await;
    }
}

async fn stream_engine(
    core: &HostCore,
    ticket: &mut Ticket,
    handle: &crate::engine::EngineHandle,
    model_id: &str,
    body: Value,
    tx: &mpsc::Sender<Out>,
    usage: &mut Usage,
) -> Result<(), String> {
    let send = core
        .engine_http
        .post(format!("{}/v1/chat/completions", handle.base_url))
        .bearer_auth(&handle.api_key)
        .json(&body)
        .send();
    let resp = tokio::select! {
        r = send => r.map_err(|_| "The host's engine stopped unexpectedly".to_string())?,
        _ = tx.closed() => return Ok(()),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        log::warn!("engine refused a request: {status}");
        return Err(if text.contains("context") || text.contains("exceed") {
            "This conversation is too long for the host's settings. Start a new one.".into()
        } else {
            "The host's engine couldn't answer this".into()
        });
    }
    let mut bytes = resp.bytes_stream();
    let mut buf = String::new();
    let mut cut_at: Option<(tokio::time::Instant, String)> = None;
    loop {
        let deadline = cut_at.as_ref().map(|c| c.0);
        tokio::select! {
            chunk = bytes.next() => {
                let Some(chunk) = chunk else { return Ok(()) };
                let chunk = chunk.map_err(|_| "The host's engine stopped unexpectedly".to_string())?;
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(i) = buf.find("\n\n") {
                    let frame: String = buf.drain(..i + 2).collect();
                    for line in frame.lines() {
                        let Some(data) = line.strip_prefix("data: ") else { continue };
                        if data.trim() == "[DONE]" {
                            return Ok(());
                        }
                        let Ok(mut v) = serde_json::from_str::<Value>(data) else { continue };
                        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                            usage.tokens_in = u["prompt_tokens"].as_u64().unwrap_or(0);
                            usage.tokens_out = u["completion_tokens"].as_u64().unwrap_or(0);
                        }
                        // the engine knows the model by its alias; never let a file path through
                        v["model"] = json!(model_id);
                        if let Some(o) = v.as_object_mut() {
                            o.remove("timings");
                            o.remove("system_fingerprint");
                        }
                        if tx.send(Out::Chunk(v)).await.is_err() {
                            return Ok(()); // client went away: dropping the stream cancels the engine
                        }
                    }
                }
            }
            r = ticket.rx.changed(), if cut_at.is_none() => {
                if r.is_err() { return Ok(()) }
                let turn = ticket.rx.borrow_and_update().clone();
                if let Turn::Stopped(reason) = turn {
                    cut_at = Some((tokio::time::Instant::now() + PAUSE_GRACE, reason));
                }
            }
            _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                return Err(cut_at.take().unwrap().1);
            }
            _ = tx.closed() => return Ok(()),
        }
    }
}
