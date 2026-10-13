//! The client's running state (spec §6.9): the hosts it can see, the chat with one of them, and
//! the turn its own question is in. The UI reads [`ClientRuntime::view`]; discovery (Phase 3)
//! and pairing (Phase 4) feed [`ClientRuntime::upsert_host`].

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use netcore::tls::ServerPin;
use netcore::Identity;
use protocol::api::{ChatMessage, ChatRequest, ErrorKind, MessageContent, QueueEvent};
use protocol::discovery::{Announcement, ShareStatus};
use serde::Serialize;

use crate::api::{ChatEvent, ClientError, HostClient};
use crate::history::{History, KnownHost};

/// How many earlier messages go with each question. The host's context size is the real limit.
const CONTEXT_MESSAGES: usize = 12;
const VIEW_MESSAGES: usize = 200;

#[derive(Debug, Clone)]
pub struct SeenHost {
    pub info: Announcement,
    pub address: SocketAddr,
    pub last_seen: Instant,
    pub models: Vec<RemoteModel>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RemoteModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum Turn {
    Queued { position: u32 },
    Loading { model: String, pct: u8 },
    Streaming,
}

#[derive(Default)]
struct State {
    hosts: BTreeMap<String, SeenHost>,
    pairing: Option<(String, String)>,
    chat_host: Option<String>,
    model: String,
    turn: Option<Turn>,
    /// The reply being written right now (saved to history when it ends).
    partial: Option<String>,
    task: Option<tokio::task::JoinHandle<()>>,
    network_hint: String,
}

pub struct ClientRuntime {
    pub identity: Identity,
    pub history: History,
    /// This computer's name, shown in the owner's pairing popup.
    pub name: String,
    state: Mutex<State>,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

impl ClientRuntime {
    pub fn new(identity: Identity, history: History, name: String) -> Arc<Self> {
        Arc::new(Self { identity, history, name, state: Mutex::new(State::default()) })
    }

    /// A client for talking to a host: pinned to its fingerprint, with our token if paired.
    pub fn api(&self, host_id: &str, address: SocketAddr) -> Result<HostClient, ClientError> {
        let token = self.history.host(host_id).ok().flatten().map(|k| k.token);
        HostClient::new(&self.identity, address, ServerPin::Fingerprint(host_id.to_string()), token)
    }

    pub fn is_paired(&self, host_id: &str) -> bool {
        self.history.host(host_id).ok().flatten().is_some()
    }

    /// Adds or refreshes a host that announced itself or answered `/api/info`. An announcement
    /// without a status means sharing is off: the host disappears.
    pub fn upsert_host(&self, info: Announcement, address: SocketAddr) {
        let mut st = self.state.lock().unwrap();
        if info.status.is_none() || !info.modes.contains(&protocol::discovery::Mode::Share) {
            st.hosts.remove(&info.id);
            return;
        }
        if self.is_paired(&info.id) {
            let _ = self.history.update_address(&info.id, &info.name, &address.to_string());
        }
        let models = st.hosts.get(&info.id).map(|h| h.models.clone()).unwrap_or_default();
        st.hosts.insert(info.id.clone(), SeenHost { info, address, last_seen: Instant::now(), models });
    }

    pub fn set_models(&self, host_id: &str, models: Vec<RemoteModel>) {
        if let Some(h) = self.state.lock().unwrap().hosts.get_mut(host_id) {
            h.models = models;
        }
    }

    /// Hosts not heard from in `max_age` drop off the list (spec §6.6: three missed heartbeats).
    pub fn expire(&self, max_age: Duration) {
        self.state.lock().unwrap().hosts.retain(|_, h| h.last_seen.elapsed() <= max_age);
    }

    pub fn hosts(&self) -> Vec<SeenHost> {
        self.state.lock().unwrap().hosts.values().cloned().collect()
    }

    pub fn set_network_hint(&self, hint: String) {
        self.state.lock().unwrap().network_hint = hint;
    }

    pub fn set_pairing(&self, p: Option<(String, String)>) {
        self.state.lock().unwrap().pairing = p;
    }

    pub fn pairing(&self) -> Option<(String, String)> {
        self.state.lock().unwrap().pairing.clone()
    }

    pub fn open_chat(&self, host_id: &str) {
        self.state.lock().unwrap().chat_host = Some(host_id.to_string());
    }

    pub fn set_model(&self, id: &str) {
        self.state.lock().unwrap().model = id.to_string();
    }

    pub fn forget(&self, host_id: &str) {
        let _ = self.history.forget_host(host_id);
        let mut st = self.state.lock().unwrap();
        if st.chat_host.as_deref() == Some(host_id) {
            st.chat_host = None;
        }
    }

    pub fn save_pairing(&self, host: &SeenHost, token: String) {
        let _ = self.history.save_host(&KnownHost {
            id: host.info.id.clone(),
            name: host.info.name.clone(),
            token,
            address: host.address.to_string(),
        });
    }

    fn note(&self, host_id: &str, text: &str) {
        let _ = self.history.add(host_id, "note", text, now_unix());
    }

    /// Sends a question to the open chat's host. The reply streams into the view.
    pub fn send(self: &Arc<Self>, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let (host, model) = {
            let st = self.state.lock().unwrap();
            if st.turn.is_some() {
                return; // one question at a time
            }
            let Some(h) = st.chat_host.as_ref().and_then(|id| st.hosts.get(id)).cloned() else { return };
            (h, st.model.clone())
        };
        let host_id = host.info.id.clone();
        let earlier = self.history.recent(&host_id, CONTEXT_MESSAGES).unwrap_or_default();
        let _ = self.history.add(&host_id, "user", &text, now_unix());
        let mut messages: Vec<ChatMessage> = earlier
            .into_iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| ChatMessage { role: m.role, content: MessageContent::Text(m.text) })
            .collect();
        messages.push(ChatMessage { role: "user".into(), content: MessageContent::Text(text) });
        let req = ChatRequest { model, messages, ..Default::default() };

        let me = self.clone();
        let mut st = self.state.lock().unwrap();
        st.turn = Some(Turn::Queued { position: 1 });
        st.partial = None;
        st.task = Some(tokio::spawn(async move { me.run_chat(host, req).await }));
    }

    async fn run_chat(self: Arc<Self>, host: SeenHost, req: ChatRequest) {
        let host_id = host.info.id.clone();
        let result = async {
            let api = self.api(&host_id, host.address)?;
            let mut stream = api.chat(&req).await?;
            while let Some(ev) = stream.next().await {
                match ev? {
                    ChatEvent::Queue(QueueEvent::Queued { position }) => self.set_turn(Some(Turn::Queued { position })),
                    ChatEvent::Queue(QueueEvent::Loading { model, pct }) => self.set_turn(Some(Turn::Loading { model, pct })),
                    ChatEvent::Queue(QueueEvent::Started) => {
                        let mut st = self.state.lock().unwrap();
                        st.turn = Some(Turn::Streaming);
                        st.partial = Some(String::new());
                    }
                    ChatEvent::Queue(QueueEvent::Stopped { reason }) => {
                        let started = self.finish_partial(&host_id);
                        let text = if started {
                            format!("{}'s computer stopped part way through. {reason}.", host.info.name)
                        } else {
                            format!("Your question wasn't answered. {reason}. Send it again when the host is back.")
                        };
                        self.note(&host_id, &text);
                    }
                    ChatEvent::Delta(t) => {
                        if let Some(p) = self.state.lock().unwrap().partial.as_mut() {
                            p.push_str(&t);
                        }
                    }
                    ChatEvent::Usage { .. } => {}
                }
            }
            Ok::<_, ClientError>(())
        }
        .await;
        self.finish_partial(&host_id);
        if let Err(e) = result {
            let text = match e.api_kind() {
                Some(ErrorKind::Paused) => format!("Your question wasn't answered. {e}. Send it again when the host is back."),
                Some(ErrorKind::AlreadyQueued) => "You already have a question waiting.".to_string(),
                Some(ErrorKind::Unauthorized) => "This computer is no longer allowed to use that host. Ask to connect again.".to_string(),
                Some(_) => format!("The host couldn't answer: {e}."),
                None => format!("Couldn't reach {}. It may have gone to sleep or left the network.", host.info.name),
            };
            self.note(&host_id, &text);
        }
        let mut st = self.state.lock().unwrap();
        st.turn = None;
        st.task = None;
    }

    fn set_turn(&self, t: Option<Turn>) {
        self.state.lock().unwrap().turn = t;
    }

    /// Saves the reply written so far to history. Returns true if one had started.
    fn finish_partial(&self, host_id: &str) -> bool {
        let partial = self.state.lock().unwrap().partial.take();
        match partial {
            Some(p) => {
                if !p.trim().is_empty() {
                    let _ = self.history.add(host_id, "assistant", &p, now_unix());
                }
                true
            }
            None => false,
        }
    }

    /// Stop the answer, or cancel a question still in line. Dropping the stream tells the host.
    pub fn stop(&self) {
        let (task, host_id, was_streaming) = {
            let mut st = self.state.lock().unwrap();
            let streaming = st.turn == Some(Turn::Streaming);
            (st.task.take(), st.chat_host.clone(), streaming)
        };
        let Some(task) = task else { return };
        task.abort();
        if let Some(host_id) = host_id {
            self.finish_partial(&host_id);
            self.note(&host_id, if was_streaming { "You stopped the answer." } else { "You cancelled your question." });
        }
        self.set_turn(None);
    }

    /// Asks every host on the list (and every paired host at its last address) how it is, and
    /// fetches the model list from paired ones. Covers networks where multicast doesn't arrive.
    pub async fn refresh(self: &Arc<Self>) {
        let mut targets: BTreeMap<String, SocketAddr> =
            self.hosts().into_iter().map(|h| (h.info.id.clone(), h.address)).collect();
        for k in self.history.hosts().unwrap_or_default() {
            if let Ok(addr) = k.address.parse() {
                targets.entry(k.id).or_insert(addr);
            }
        }
        let jobs = targets.into_iter().map(|(id, addr)| {
            let me = self.clone();
            async move {
                let Ok(api) = me.api(&id, addr) else { return };
                let Ok(info) = api.info().await else { return };
                if info.id != id {
                    return;
                }
                let paired = me.is_paired(&id);
                me.upsert_host(info, addr);
                if paired {
                    match api.models().await {
                        Ok(list) => me.set_models(
                            &id,
                            list.data.into_iter().map(|m| RemoteModel { name: if m.name.is_empty() { m.id.clone() } else { m.name }, id: m.id }).collect(),
                        ),
                        Err(e) if e.api_kind() == Some(ErrorKind::Unauthorized) => {
                            // the owner removed us
                            let _ = me.history.forget_host(&id);
                        }
                        Err(_) => {}
                    }
                }
            }
        });
        futures_util::future::join_all(jobs).await;
    }

    /// "Add by IP address" (`P3-T5`), for networks where computers can't find each other. The
    /// fingerprint isn't known yet, so the first contact records whatever certificate answers
    /// and checks it is the one the host claims; the pairing code guards the rest.
    pub async fn add_by_ip(self: &Arc<Self>, input: &str) -> Result<String, String> {
        let input = input.trim();
        let addr: SocketAddr = match input.parse() {
            Ok(a) => a,
            Err(_) => match input.parse::<std::net::IpAddr>() {
                Ok(ip) => SocketAddr::new(ip, protocol::GATEKEEPER_PORT),
                Err(_) => return Err("That doesn't look like an address. Try something like 192.168.1.42.".into()),
            },
        };
        let seen = Arc::new(Mutex::new(None));
        let api = HostClient::new(&self.identity, addr, ServerPin::Capture(seen.clone()), None).map_err(|e| e.to_string())?;
        let info = api.info().await.map_err(|_| {
            format!("Nothing answered at {addr}. Check the address, and that sharing is switched on there.")
        })?;
        if seen.lock().unwrap().as_deref() != Some(info.id.as_str()) {
            return Err("That computer's identity didn't check out, so it wasn't added.".into());
        }
        if info.status.is_none() {
            return Err("That computer isn't sharing right now.".into());
        }
        let name = info.name.clone();
        self.upsert_host(info, addr);
        Ok(name)
    }

    /// The Use-mode screens (`ClientView` in the UI).
    pub fn view(&self) -> ClientView {
        let st = self.state.lock().unwrap();
        let hosts = st
            .hosts
            .values()
            .map(|h| {
                let paired = self.is_paired(&h.info.id);
                let mine = st.chat_host.as_deref() == Some(h.info.id.as_str()) && st.turn.is_some();
                ViewHost {
                    id: h.info.id.clone(),
                    name: h.info.name.clone(),
                    os: match h.info.os {
                        protocol::discovery::Os::Windows => "Windows".into(),
                        protocol::discovery::Os::Linux => "Linux".into(),
                    },
                    gpu: h.info.gpu.clone().unwrap_or_default(),
                    status: match h.info.status {
                        Some(ShareStatus::Available) => "available",
                        Some(ShareStatus::Busy) => "busy",
                        Some(ShareStatus::Paused) => "paused",
                        None => "off",
                    }
                    .into(),
                    reason: h.info.reason.clone().unwrap_or_default(),
                    busy_with_other: h.info.status == Some(ShareStatus::Busy) && !mine,
                    paired,
                    models: if paired { h.models.clone() } else { vec![] },
                    address: h.address.to_string(),
                }
            })
            .collect();
        let mut msgs: Vec<ViewMessage> = st
            .chat_host
            .as_ref()
            .map(|id| self.history.recent(id, VIEW_MESSAGES).unwrap_or_default())
            .unwrap_or_default()
            .into_iter()
            .map(|m| ViewMessage { role: m.role, text: m.text, streaming: false })
            .collect();
        if let Some(p) = &st.partial {
            msgs.push(ViewMessage { role: "assistant".into(), text: p.clone(), streaming: true });
        }
        ClientView {
            scanning: true,
            hosts,
            pairing: st.pairing.as_ref().map(|(host_id, code)| ViewPairing { host_id: host_id.clone(), code: code.clone() }),
            chat_host: st.chat_host.clone(),
            model: st.model.clone(),
            msgs,
            turn: st.turn.clone(),
            network_hint: st.network_hint.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub scanning: bool,
    pub hosts: Vec<ViewHost>,
    pub pairing: Option<ViewPairing>,
    pub chat_host: Option<String>,
    pub model: String,
    pub msgs: Vec<ViewMessage>,
    pub turn: Option<Turn>,
    pub network_hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewHost {
    pub id: String,
    pub name: String,
    pub os: String,
    pub gpu: String,
    pub status: String,
    pub reason: String,
    pub busy_with_other: bool,
    pub paired: bool,
    pub models: Vec<RemoteModel>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPairing {
    pub host_id: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewMessage {
    pub role: String,
    pub text: String,
    pub streaming: bool,
}
