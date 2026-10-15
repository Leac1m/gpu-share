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
use protocol::api::{ChatMessage, ChatRequest, ErrorKind, QueueEvent};
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
    /// Polls in a row that got no answer.
    pub fails: u32,
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
    /// A one-off message for the Computers screen ("The owner said not now.").
    notice: String,
    /// The local API's settings and state, for the Chat screens (`P9-T2`).
    local_api: LocalApiView,
}

/// What the UI shows about the local API (`P9-T2`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalApiView {
    pub enabled: bool,
    /// The base URL to give other programs, e.g. `http://127.0.0.1:50125/v1`.
    pub url: String,
    pub key: String,
    /// Why it couldn't start (port in use).
    pub error: String,
}

pub struct ClientRuntime {
    pub identity: Identity,
    pub history: History,
    /// This computer's name, shown in the owner's pairing popup.
    pub name: String,
    state: Mutex<State>,
    /// This computer's own Share mode, if it has one (`P9-T1`): its loopback listener and the
    /// in-memory token for it.
    own: Mutex<Option<(SocketAddr, String)>>,
}

/// What the owner's own computer is called in his Use mode.
pub const OWN_NAME: &str = "This computer";

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

impl ClientRuntime {
    pub fn new(identity: Identity, history: History, name: String) -> Arc<Self> {
        Arc::new(Self { identity, history, name, state: Mutex::new(State::default()), own: Mutex::new(None) })
    }

    /// Adds this computer's own Share mode to the list as "This computer" (`P9-T1`): always
    /// there, always paired, reached over its loopback listener.
    pub fn set_own_host(&self, address: SocketAddr, token: String, info: Announcement) {
        *self.own.lock().unwrap() = Some((address, token));
        self.upsert_host(info, address);
    }

    /// Whether `host_id` is this computer's own Share mode.
    pub fn is_own(&self, host_id: &str) -> bool {
        host_id == self.identity.fingerprint() && self.own.lock().unwrap().is_some()
    }

    /// A client for talking to a host: pinned to its fingerprint, with our token if paired.
    pub fn api(&self, host_id: &str, address: SocketAddr) -> Result<HostClient, ClientError> {
        let token = if self.is_own(host_id) {
            self.own.lock().unwrap().as_ref().map(|(_, t)| t.clone())
        } else {
            self.history.host(host_id).ok().flatten().map(|k| k.token)
        };
        HostClient::new(&self.identity, address, ServerPin::Fingerprint(host_id.to_string()), token)
    }

    pub fn is_paired(&self, host_id: &str) -> bool {
        self.is_own(host_id) || self.history.host(host_id).ok().flatten().is_some()
    }

    /// Adds or refreshes a host that announced itself or answered `/api/info`. An announcement
    /// without a status means sharing is off: the host disappears.
    pub fn upsert_host(&self, mut info: Announcement, address: SocketAddr) {
        if self.is_own(&info.id) {
            // the owner's own computer answers him whether sharing is on, off or paused
            info.name = OWN_NAME.into();
            info.status = Some(if info.status == Some(ShareStatus::Busy) { ShareStatus::Busy } else { ShareStatus::Available });
            info.reason = None;
            let mut st = self.state.lock().unwrap();
            let models = st.hosts.get(&info.id).map(|h| h.models.clone()).unwrap_or_default();
            st.hosts.insert(info.id.clone(), SeenHost { info, address, last_seen: Instant::now(), models, fails: 0 });
            return;
        }
        let mut st = self.state.lock().unwrap();
        if info.status.is_none() || !info.modes.contains(&protocol::discovery::Mode::Share) {
            st.hosts.remove(&info.id);
            return;
        }
        if self.is_paired(&info.id) {
            let _ = self.history.update_address(&info.id, &info.name, &address.to_string());
        }
        if !protocol::compatible(&info.protocol) {
            log::info!("{} speaks protocol {}, this build {}", info.name, info.protocol, protocol::PROTOCOL_VERSION);
        }
        let models = st.hosts.get(&info.id).map(|h| h.models.clone()).unwrap_or_default();
        st.hosts.insert(info.id.clone(), SeenHost { info, address, last_seen: Instant::now(), models, fails: 0 });
    }

    pub fn set_models(&self, host_id: &str, models: Vec<RemoteModel>) {
        if let Some(h) = self.state.lock().unwrap().hosts.get_mut(host_id) {
            h.models = models;
        }
    }

    /// A poll got no answer. Two in a row (about ten seconds) and the host is gone: its owner
    /// switched sharing off, or it left the network.
    fn missed(&self, id: &str) {
        if self.is_own(id) {
            return;
        }
        let mut st = self.state.lock().unwrap();
        let gone = match st.hosts.get_mut(id) {
            Some(h) => {
                h.fails += 1;
                h.fails >= 2
            }
            None => false,
        };
        if gone {
            st.hosts.remove(id);
        }
    }

    /// Hosts not heard from in `max_age` drop off the list (spec §6.6: three missed heartbeats).
    pub fn expire(&self, max_age: Duration) {
        let own = self.own.lock().unwrap().is_some().then(|| self.identity.fingerprint().to_string());
        self.state.lock().unwrap().hosts.retain(|id, h| Some(id) == own.as_ref() || h.last_seen.elapsed() <= max_age);
    }

    pub fn hosts(&self) -> Vec<SeenHost> {
        self.state.lock().unwrap().hosts.values().cloned().collect()
    }

    pub fn set_local_api(&self, v: LocalApiView) {
        self.state.lock().unwrap().local_api = v;
    }

    /// Where the local API sends a request for `model`: paired hosts (and this computer) that
    /// offer it, this computer first, then the computer the chat is open with, then the rest.
    pub fn route_model(&self, model: &str) -> Option<SeenHost> {
        self.api_hosts().into_iter().find(|h| h.models.iter().any(|m| m.id == model))
    }

    /// Paired hosts with their models, in the order the local API prefers them.
    pub fn api_hosts(&self) -> Vec<SeenHost> {
        let st = self.state.lock().unwrap();
        let chat = st.chat_host.clone();
        let mut list: Vec<SeenHost> = st.hosts.values().filter(|h| self.is_paired(&h.info.id)).cloned().collect();
        list.sort_by_key(|h| (!self.is_own(&h.info.id), chat.as_deref() != Some(h.info.id.as_str())));
        list
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

    pub fn set_notice(&self, text: impl Into<String>) {
        self.state.lock().unwrap().notice = text.into();
    }

    /// "Ask to connect" (`P4-T3`): shows the code at once and waits for the owner. The code is
    /// computed here from the fingerprint we pinned, our own and a fresh nonce; the host computes
    /// it from what it sees, so the two only match without a man in the middle.
    pub fn ask_connect(self: &Arc<Self>, host_id: &str) {
        let Some(host) = self.hosts().into_iter().find(|h| h.info.id == host_id) else { return };
        let nonce = crate::api::new_nonce();
        let code = protocol::pairing::pairing_code(host_id, self.identity.fingerprint(), &nonce);
        {
            let mut st = self.state.lock().unwrap();
            if st.pairing.is_some() {
                return;
            }
            st.pairing = Some((host_id.to_string(), code));
            st.notice.clear();
        }
        let me = self.clone();
        tokio::spawn(async move {
            let req = protocol::pairing::PairRequest { protocol: protocol::PROTOCOL_VERSION.into(), name: me.name.clone(), nonce };
            let result = async { me.api(&host.info.id, host.address)?.pair(&req).await }.await;
            match result {
                Ok(resp) => {
                    me.save_pairing(&host, resp.token);
                    me.refresh().await; // fetch the model list now
                    me.open_chat(&host.info.id);
                }
                Err(e) => me.set_notice(match e.api_kind() {
                    Some(_) => e.to_string(),
                    None => format!("Couldn't reach {}. Try again.", host.info.name),
                }),
            }
            // last, so the screen sees the chat open by the time the code disappears
            me.set_pairing(None);
        });
    }

    pub fn open_chat(&self, host_id: &str) {
        self.state.lock().unwrap().chat_host = Some(host_id.to_string());
    }

    pub fn set_model(&self, id: &str) {
        self.state.lock().unwrap().model = id.to_string();
    }

    pub fn forget(&self, host_id: &str) {
        if self.is_own(host_id) {
            return;
        }
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
            .map(|m| ChatMessage::text(m.role, m.text))
            .collect();
        messages.push(ChatMessage::text("user", text));
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
                        let text = if started && self.is_own(&host_id) {
                            format!("This computer stopped part way through. {reason}.")
                        } else if started {
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

    /// "Ask again" after a pause cut a reply short or turned the question away (`P5-T10`).
    pub fn retry(self: &Arc<Self>) {
        let Some(host_id) = self.state.lock().unwrap().chat_host.clone() else { return };
        let last_user = self.history.recent(&host_id, 50).unwrap_or_default().into_iter().rev().find(|m| m.role == "user");
        if let Some(m) = last_user {
            self.send(&m.text);
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
                let Ok(info) = api.info().await else {
                    me.missed(&id);
                    return;
                };
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
                            // the owner removed us; keep our own copy of the conversation
                            let _ = me.history.unpair_host(&id);
                            me.set_models(&id, vec![]);
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
        let own_id = self.own.lock().unwrap().is_some().then(|| self.identity.fingerprint().to_string());
        let mut hosts: Vec<ViewHost> = st
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
                        None => "off",
                        Some(_) if !protocol::compatible(&h.info.protocol) => "paused",
                        Some(ShareStatus::Available) => "available",
                        Some(ShareStatus::Busy) => "busy",
                        Some(ShareStatus::Paused) => "paused",
                    }
                    .into(),
                    reason: if protocol::compatible(&h.info.protocol) {
                        h.info.reason.clone().unwrap_or_default()
                    } else {
                        "Runs a different version of GPU Share. Update both computers to the same version.".into()
                    },
                    busy_with_other: h.info.status == Some(ShareStatus::Busy) && !mine,
                    paired,
                    models: if paired { h.models.clone() } else { vec![] },
                    address: h.address.to_string(),
                    own: own_id.as_deref() == Some(h.info.id.as_str()),
                }
            })
            .collect();
        hosts.sort_by_key(|h| !h.own); // "This computer" first
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
            notice: st.notice.clone(),
            local_api: st.local_api.clone(),
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
    pub notice: String,
    pub local_api: LocalApiView,
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
    /// The owner's own computer (`P9-T1`).
    pub own: bool,
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
