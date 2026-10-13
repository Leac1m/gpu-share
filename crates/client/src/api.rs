//! Talking to one host's gatekeeper over pinned TLS.

use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use netcore::tls::ServerPin;
use netcore::Identity;
use protocol::api::{ApiError, ChatRequest, ModelList, QueueEvent, QUEUE_EVENT};
use protocol::discovery::Announcement;
use protocol::pairing::{PairRequest, PairResponse};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Couldn't reach the host, or its certificate wasn't the one we expected.
    #[error("couldn't reach the computer: {0}")]
    Network(String),
    /// The host answered with an error.
    #[error("{}", .1.error.message)]
    Api(u16, ApiError),
    #[error("unexpected answer from the computer: {0}")]
    Protocol(String),
}

impl ClientError {
    pub fn api_kind(&self) -> Option<protocol::api::ErrorKind> {
        match self {
            ClientError::Api(_, e) => Some(e.error.kind),
            _ => None,
        }
    }
}

/// What a chat stream yields.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    Queue(QueueEvent),
    /// A piece of the reply.
    Delta(String),
    Usage { prompt_tokens: u64, completion_tokens: u64 },
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatEvent, ClientError>> + Send>>;

#[derive(Clone)]
pub struct HostClient {
    base: String,
    http: reqwest::Client,
    token: Option<String>,
}

impl HostClient {
    /// `pin` is the host's fingerprint as announced (or [`ServerPin::Capture`] for a first
    /// contact by IP). `token` is set once paired.
    pub fn new(identity: &Identity, addr: SocketAddr, pin: ServerPin, token: Option<String>) -> Result<Self, ClientError> {
        let tls = netcore::tls::client_config(identity, pin).map_err(|e| ClientError::Network(e.to_string()))?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(Self { base: format!("https://{addr}"), http, token })
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let r = self.http.request(m, format!("{}{path}", self.base));
        match &self.token {
            Some(t) => r.bearer_auth(t),
            None => r,
        }
    }

    async fn json<T: serde::de::DeserializeOwned>(r: reqwest::RequestBuilder) -> Result<T, ClientError> {
        let resp = r.send().await.map_err(|e| ClientError::Network(e.to_string()))?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| ClientError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(api_error(status.as_u16(), &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub async fn info(&self) -> Result<Announcement, ClientError> {
        Self::json(self.req(reqwest::Method::GET, "/api/info").timeout(Duration::from_secs(5))).await
    }

    pub async fn register(&self, me: &Announcement) -> Result<Announcement, ClientError> {
        Self::json(self.req(reqwest::Method::POST, "/api/register").json(me).timeout(Duration::from_secs(5))).await
    }

    /// Waits until the owner answers (or the host gives up), so no timeout here beyond the host's.
    pub async fn pair(&self, req: &PairRequest) -> Result<PairResponse, ClientError> {
        Self::json(self.req(reqwest::Method::POST, "/api/pair").json(req).timeout(Duration::from_secs(150))).await
    }

    pub async fn models(&self) -> Result<ModelList, ClientError> {
        Self::json(self.req(reqwest::Method::GET, "/v1/models").timeout(Duration::from_secs(10))).await
    }

    /// Starts a streaming chat. Dropping the returned stream cancels the request on the host.
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatStream, ClientError> {
        let mut req = req.clone();
        req.stream = Some(true);
        let resp = self
            .req(reqwest::Method::POST, "/v1/chat/completions")
            .json(&req)
            .send()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let bytes = resp.bytes().await.unwrap_or_default();
            return Err(api_error(status.as_u16(), &bytes));
        }
        let bytes = resp.bytes_stream();
        let s = futures_util::stream::unfold(
            (bytes, String::new(), std::collections::VecDeque::new(), false),
            |(mut bytes, mut buf, mut ready, mut done)| async move {
                loop {
                    if let Some(ev) = ready.pop_front() {
                        return Some((ev, (bytes, buf, ready, done)));
                    }
                    if done {
                        return None;
                    }
                    match bytes.next().await {
                        None => return None,
                        Some(Err(e)) => {
                            done = true;
                            ready.push_back(Err(ClientError::Network(e.to_string())));
                        }
                        Some(Ok(chunk)) => {
                            buf.push_str(&String::from_utf8_lossy(&chunk));
                            while let Some(i) = buf.find("\n\n") {
                                let frame: String = buf.drain(..i + 2).collect();
                                match parse_frame(&frame) {
                                    Frame::Events(evs) => ready.extend(evs.into_iter().map(Ok)),
                                    Frame::Done => done = true,
                                    Frame::Skip => {}
                                }
                            }
                        }
                    }
                }
            },
        );
        Ok(Box::pin(s))
    }
}

fn api_error(status: u16, bytes: &[u8]) -> ClientError {
    match serde_json::from_slice::<ApiError>(bytes) {
        Ok(e) => ClientError::Api(status, e),
        Err(_) => ClientError::Protocol(format!("HTTP {status}")),
    }
}

enum Frame {
    Events(Vec<ChatEvent>),
    Done,
    Skip,
}

fn parse_frame(frame: &str) -> Frame {
    let mut event = None;
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(e) = line.strip_prefix("event:") {
            event = Some(e.trim().to_string());
        } else if let Some(d) = line.strip_prefix("data:") {
            data.push_str(d.trim_start());
        }
    }
    if data.trim() == "[DONE]" {
        return Frame::Done;
    }
    if event.as_deref() == Some(QUEUE_EVENT) {
        return match serde_json::from_str::<QueueEvent>(&data) {
            Ok(q) => Frame::Events(vec![ChatEvent::Queue(q)]),
            Err(_) => Frame::Skip,
        };
    }
    let Ok(v) = serde_json::from_str::<Value>(&data) else { return Frame::Skip };
    let mut out = vec![];
    if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
        if !t.is_empty() {
            out.push(ChatEvent::Delta(t.to_string()));
        }
    }
    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
        out.push(ChatEvent::Usage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
        });
    }
    Frame::Events(out)
}

/// A fresh 16-byte nonce for pairing, as 32 hex characters.
pub fn new_nonce() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(f: &str) -> Vec<ChatEvent> {
        match parse_frame(f) {
            Frame::Events(e) => e,
            _ => vec![],
        }
    }

    #[test]
    fn parses_queue_events_and_deltas() {
        assert_eq!(
            events("event: gpushare\ndata: {\"type\":\"queued\",\"position\":2}\n\n"),
            vec![ChatEvent::Queue(QueueEvent::Queued { position: 2 })]
        );
        assert_eq!(
            events("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n"),
            vec![ChatEvent::Delta("Hi".into())]
        );
        assert!(matches!(parse_frame("data: [DONE]\n\n"), Frame::Done));
    }

    #[test]
    fn the_usage_chunk_is_reported() {
        assert_eq!(
            events("data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":7}}\n\n"),
            vec![ChatEvent::Usage { prompt_tokens: 5, completion_tokens: 7 }]
        );
    }

    #[test]
    fn unknown_named_events_are_ignored() {
        assert!(events("event: other\ndata: {\"x\":1}\n\n").is_empty());
    }
}
