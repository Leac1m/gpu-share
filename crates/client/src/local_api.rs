//! The local API (`P9-T2`, spec §6.9): an OpenAI-compatible address on this computer, so
//! editors, scripts and agent harnesses can use a paired host's model (or this computer's own)
//! as if it were any OpenAI-style server.
//!
//! * Listens on `127.0.0.1` only, and only while the user has switched it on.
//! * Every request needs the key shown in the app (`Authorization: Bearer …`), and a `Host`
//!   header naming this address (so a web page can't reach it through DNS rebinding). No CORS
//!   headers are sent, so browsers can't read its answers.
//! * `GET /v1/models` and `POST /v1/chat/completions`, nothing else. Requests go to a paired
//!   host over the same pinned, authenticated connection the chat screen uses, so they wait in
//!   that host's line and follow its owner's limits like any of this computer's questions.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use protocol::api::{ChatRequest, CompletionBuilder, ErrorKind, QueueEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::api::{ClientError, RawEvent};
use crate::runtime::{ClientRuntime, LocalApiView};

/// The default port: next to the gatekeeper's 50123, easy to put in a harness's settings.
pub const DEFAULT_PORT: u16 = 50125;
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Kept in the data folder, so a harness's settings stay valid across restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalApiSettings {
    pub enabled: bool,
    pub port: u16,
    pub key: String,
}

impl LocalApiSettings {
    /// The saved settings, or new ones (off, a fresh key) if there are none.
    pub fn load_or_new(path: &Path) -> Self {
        let saved: Option<Self> = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        saved.filter(|s| s.key.len() >= 32).unwrap_or_else(|| Self { enabled: false, port: DEFAULT_PORT, key: new_key() })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap())
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// What the UI shows.
    pub fn view(&self, error: impl Into<String>) -> LocalApiView {
        LocalApiView { enabled: self.enabled, url: self.url(), key: self.key.clone(), error: error.into() }
    }
}

/// `gs-` and 48 hex characters.
pub fn new_key() -> String {
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::rng().fill_bytes(&mut b);
    format!("gs-{}", hex::encode(b))
}

/// A running local API; dropping it (or [`LocalApi::stop`]) closes the port.
pub struct LocalApi {
    pub addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Clone)]
struct Ctx {
    rt: Arc<ClientRuntime>,
    key: Arc<String>,
    port: u16,
}

impl LocalApi {
    /// Starts on `127.0.0.1:port` (`0` picks a free port, for tests).
    pub async fn start(rt: Arc<ClientRuntime>, port: u16, key: String) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
        let addr = listener.local_addr()?;
        let ctx = Ctx { rt, key: Arc::new(key), port: addr.port() };
        let app = Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat))
            .fallback(|| async { openai_error(StatusCode::NOT_FOUND, "not_found", "Only /v1/models and /v1/chat/completions are here.") })
            .layer(middleware::from_fn_with_state(ctx.clone(), guard))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(ctx);
        let (shutdown, mut stop) = watch::channel(false);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stop.changed().await;
                })
                .await;
        });
        Ok(Self { addr, shutdown, task: Some(task) })
    }

    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(t) = self.task.take() {
            t.abort();
            let _ = t.await;
        }
    }
}

impl Drop for LocalApi {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

fn openai_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let kind = if status.is_server_error() { "server_error" } else { "invalid_request_error" };
    (status, Json(json!({ "error": { "message": message.into(), "type": kind, "code": code } }))).into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Only this address by name (no DNS rebinding), and only with the key.
async fn guard(State(ctx): State<Ctx>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let allowed = [format!("127.0.0.1:{}", ctx.port), format!("localhost:{}", ctx.port)];
    if !allowed.iter().any(|a| a.eq_ignore_ascii_case(host)) {
        return openai_error(StatusCode::FORBIDDEN, "bad_host", "Use the address GPU Share shows, 127.0.0.1 or localhost.");
    }
    let key = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    if !constant_time_eq(key.as_bytes(), ctx.key.as_bytes()) {
        return openai_error(StatusCode::UNAUTHORIZED, "invalid_api_key", "Use the API key shown in GPU Share.");
    }
    next.run(req).await
}

/// Every model the paired computers offer, each once (the preferred computer's copy).
async fn models(State(ctx): State<Ctx>) -> Response {
    let mut seen = std::collections::HashSet::new();
    let mut data = vec![];
    for h in ctx.rt.api_hosts() {
        for m in &h.models {
            if seen.insert(m.id.clone()) {
                data.push(json!({ "id": m.id, "object": "model", "owned_by": h.info.name, "name": m.name }));
            }
        }
    }
    Json(json!({ "object": "list", "data": data })).into_response()
}

fn host_error(e: ClientError, host: &str) -> Response {
    match e.api_kind() {
        Some(ErrorKind::Paused) => openai_error(StatusCode::SERVICE_UNAVAILABLE, "host_paused", format!("{host}: {e}")),
        Some(ErrorKind::AlreadyQueued) => openai_error(
            StatusCode::TOO_MANY_REQUESTS,
            "already_waiting",
            "This computer already has a question waiting on that host. One at a time.",
        ),
        Some(ErrorKind::UnknownModel) => openai_error(StatusCode::NOT_FOUND, "model_not_found", format!("{host} no longer offers that model.")),
        Some(ErrorKind::Unauthorized) => openai_error(StatusCode::BAD_GATEWAY, "not_paired", format!("This computer is no longer paired with {host}.")),
        Some(_) => openai_error(StatusCode::BAD_REQUEST, "invalid_request", e.to_string()),
        None => openai_error(StatusCode::BAD_GATEWAY, "host_unreachable", format!("Couldn't reach {host}: {e}")),
    }
}

async fn chat(State(ctx): State<Ctx>, body: Bytes) -> Response {
    let req: ChatRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return openai_error(StatusCode::BAD_REQUEST, "invalid_request", format!("Bad request: {e}")),
    };
    let Some(host) = ctx.rt.route_model(&req.model) else {
        let ids: Vec<String> = ctx.rt.api_hosts().iter().flat_map(|h| h.models.iter().map(|m| m.id.clone())).collect();
        let hint = if ids.is_empty() { "No paired computer is offering a model right now.".to_string() } else { format!("Available: {}.", ids.join(", ")) };
        return openai_error(StatusCode::NOT_FOUND, "model_not_found", format!("No paired computer offers \"{}\". {hint}", req.model));
    };
    let name = host.info.name.clone();
    let api = match ctx.rt.api(&host.info.id, host.address) {
        Ok(a) => a,
        Err(e) => return host_error(e, &name),
    };
    let mut stream = match api.chat_raw(&req).await {
        Ok(s) => s,
        Err(e) => return host_error(e, &name),
    };

    // wait in the host's line before answering, so a pause can still be a proper error
    let mut first = vec![];
    loop {
        match stream.next().await {
            Some(Ok(RawEvent::Queue(QueueEvent::Started))) => break,
            Some(Ok(RawEvent::Queue(QueueEvent::Stopped { reason }))) => {
                return openai_error(StatusCode::SERVICE_UNAVAILABLE, "host_paused", format!("{name}: {reason}"))
            }
            Some(Ok(RawEvent::Queue(_))) => continue,
            Some(Ok(RawEvent::Data(v))) => {
                first.push(v);
                break;
            }
            Some(Err(e)) => return host_error(e, &name),
            None => return openai_error(StatusCode::BAD_GATEWAY, "host_unreachable", format!("{name} closed the connection.")),
        }
    }

    if req.stream.unwrap_or(false) {
        let head = futures_util::stream::iter(first.into_iter().map(|v| Ok::<_, std::convert::Infallible>(format!("data: {v}\n\n"))));
        let rest = stream.filter_map(|ev| async move {
            match ev {
                Ok(RawEvent::Data(v)) => Some(Ok(format!("data: {v}\n\n"))),
                Ok(RawEvent::Queue(QueueEvent::Stopped { reason })) => {
                    let err = json!({ "error": { "message": reason, "type": "server_error", "code": "host_paused" } });
                    Some(Ok(format!("data: {err}\n\n")))
                }
                Ok(RawEvent::Queue(_)) => None,
                Err(e) => {
                    let err = json!({ "error": { "message": e.to_string(), "type": "server_error", "code": "host_unreachable" } });
                    Some(Ok(format!("data: {err}\n\n")))
                }
            }
        });
        let done = futures_util::stream::once(async { Ok("data: [DONE]\n\n".to_string()) });
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(head.chain(rest).chain(done)))
            .unwrap()
    } else {
        let mut done = CompletionBuilder::default();
        for v in &first {
            done.push(v);
        }
        while let Some(ev) = stream.next().await {
            match ev {
                Ok(RawEvent::Data(v)) => done.push(&v),
                Ok(RawEvent::Queue(QueueEvent::Stopped { reason })) => {
                    return openai_error(StatusCode::SERVICE_UNAVAILABLE, "host_paused", format!("{name} stopped part way: {reason}"))
                }
                Ok(RawEvent::Queue(_)) => {}
                Err(e) => return host_error(e, &name),
            }
        }
        let mut out = done.finish(&req.model);
        out["id"] = json!(format!("chatcmpl-{}", &new_key()[3..19]));
        out["created"] = json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
        Json::<Value>(out).into_response()
    }
}
