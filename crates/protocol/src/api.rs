//! Bodies of the gatekeeper endpoints (spec §6.8) other than discovery and pairing.
//!
//! `/v1/models` and `/v1/chat/completions` follow the OpenAI shapes so ordinary clients work,
//! with one addition: while a request waits in line or a model loads, the stream carries
//! `event: gpushare` messages ([`QueueEvent`]) before the usual `data:` chunks. OpenAI clients
//! skip named events they don't know, so this stays compatible.

use serde::{Deserialize, Serialize};

/// `GET /v1/models` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelList {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

impl ModelList {
    pub fn new(data: Vec<ModelInfo>) -> Self {
        Self { object: "list".into(), data }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Catalog id, used as `model` in chat requests.
    pub id: String,
    pub object: String,
    pub owned_by: String,
    /// Display name ("Qwen3 8B"). Not in the OpenAI shape; OpenAI clients ignore it.
    #[serde(default)]
    pub name: String,
}

/// A chat message. Only text content is accepted; OpenAI's array-of-parts form is flattened
/// to its text parts by the gatekeeper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<serde_json::Value>),
}

impl MessageContent {
    /// The text of the message, with any non-text parts (images, audio) dropped.
    pub fn text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// `POST /v1/chat/completions` request, as received. Fields not listed here are dropped when
/// parsing, which is the first half of request clamping (`P2-T7`): nothing the gatekeeper
/// doesn't know about ever reaches the engine.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Stop>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Stop {
    One(String),
    Many(Vec<String>),
}

/// SSE event name for [`QueueEvent`]s.
pub const QUEUE_EVENT: &str = "gpushare";

/// What the gatekeeper tells a streaming client before and around the engine's own chunks.
/// Never says whose questions are ahead, or what they asked (spec §6.9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueueEvent {
    /// Waiting in line. `position` 2 means one request is ahead of yours.
    Queued { position: u32 },
    /// Your turn, but the model is being loaded onto the card first.
    Loading { model: String, pct: u8 },
    /// The engine has started on your request; OpenAI chunks follow.
    Started,
    /// The host stopped your request (paused, switched off, removed you). `reason` is the
    /// general, friend-facing reason.
    Stopped { reason: String },
}

/// Error body, in the OpenAI shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: ErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Not paired, or the device was removed.
    Unauthorized,
    /// The host is paused or switched off; `message` is the friend-facing reason.
    Paused,
    /// This device already has a request waiting or running (spec §6.8).
    AlreadyQueued,
    /// Model not offered by the host.
    UnknownModel,
    InvalidRequest,
    /// The engine failed to start or crashed.
    EngineFailed,
    /// Pairing: the owner is answering another request.
    Busy,
    /// Pairing: the owner said no (or didn't answer).
    Denied,
}

impl ApiError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { error: ApiErrorBody { message: message.into(), kind } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_events_match_the_golden_fixture() {
        let golden: Vec<QueueEvent> =
            serde_json::from_str(include_str!("../tests/fixtures/queue_events.json")).unwrap();
        assert_eq!(
            golden,
            vec![
                QueueEvent::Queued { position: 2 },
                QueueEvent::Loading { model: "qwen3-8b-q4km".into(), pct: 40 },
                QueueEvent::Started,
                QueueEvent::Stopped { reason: "They're using their computer".into() },
            ]
        );
        let back: serde_json::Value = serde_json::to_value(&golden).unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/queue_events.json")).unwrap();
        assert_eq!(back, fixture);
    }

    #[test]
    fn unknown_request_fields_are_dropped_on_parse() {
        let req: ChatRequest = serde_json::from_value(serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "id_slot": 0,
            "n": 8,
            "grammar": "root ::= x",
            "max_tokens": 50
        }))
        .unwrap();
        let back = serde_json::to_value(&req).unwrap();
        assert_eq!(
            back,
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 50
            })
        );
    }

    #[test]
    fn content_parts_flatten_to_their_text() {
        let m: ChatMessage = serde_json::from_value(serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "look at"},
                {"type": "image_url", "image_url": {"url": "data:..."}},
                {"type": "text", "text": "this"}
            ]
        }))
        .unwrap();
        assert_eq!(m.content.text(), "look at\nthis");
    }

    #[test]
    fn errors_use_the_openai_shape() {
        let e = ApiError::new(ErrorKind::AlreadyQueued, "You already have a question waiting.");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"error": {"message": "You already have a question waiting.", "type": "already_queued"}})
        );
    }
}
