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
///
/// Tool calling (`P9-T3`): an assistant message may carry the `tool_calls` the model asked for
/// (its `content` is then often `null`), and a `tool` message carries a tool's result back,
/// tied to the call by `tool_call_id`. The host never runs a tool; the client's harness does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub content: MessageContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: role.into(), content: MessageContent::Text(text.into()), tool_calls: None, tool_call_id: None }
    }
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<MessageContent, D::Error> {
    Ok(Option::<MessageContent>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<serde_json::Value>),
}

impl Default for MessageContent {
    fn default() -> Self {
        MessageContent::Text(String::new())
    }
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
    /// OpenAI function tools the model may call (`P9-T3`). Checked and rebuilt by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    /// `"auto"`, `"none"`, `"required"`, or `{"type":"function","function":{"name":…}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// `{"type":"text"}`, `{"type":"json_object"}`, or `{"type":"json_schema",…}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
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

/// Builds one OpenAI `chat.completion` out of streamed `chat.completion.chunk`s, for clients
/// that didn't ask to stream: the text, any tool calls (whose arguments arrive in pieces, keyed
/// by `index`), the finish reason and the usage.
#[derive(Debug, Default)]
pub struct CompletionBuilder {
    text: String,
    calls: Vec<serde_json::Map<String, serde_json::Value>>,
    finish_reason: Option<String>,
    usage: Option<serde_json::Value>,
}

impl CompletionBuilder {
    pub fn push(&mut self, chunk: &serde_json::Value) {
        use serde_json::{json, Value};
        let choice = &chunk["choices"][0];
        if let Some(t) = choice["delta"]["content"].as_str() {
            self.text.push_str(t);
        }
        for c in choice["delta"]["tool_calls"].as_array().into_iter().flatten() {
            let i = c["index"].as_u64().unwrap_or(self.calls.len() as u64) as usize;
            if i > 255 {
                continue;
            }
            while self.calls.len() <= i {
                self.calls.push(serde_json::Map::new());
            }
            let call = &mut self.calls[i];
            if let Some(id) = c["id"].as_str() {
                call.insert("id".into(), json!(id));
            }
            call.insert("type".into(), json!("function"));
            let f = call.entry("function").or_insert_with(|| json!({ "name": "", "arguments": "" }));
            if let Some(n) = c["function"]["name"].as_str() {
                let had = f["name"].as_str().unwrap_or("").to_string();
                f["name"] = Value::String(had + n);
            }
            if let Some(a) = c["function"]["arguments"].as_str() {
                let had = f["arguments"].as_str().unwrap_or("").to_string();
                f["arguments"] = Value::String(had + a);
            }
        }
        if let Some(r) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(r.to_string());
        }
        if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(u.clone());
        }
    }

    pub fn finish(self, model: &str) -> serde_json::Value {
        use serde_json::{json, Value};
        let has_calls = !self.calls.is_empty();
        let mut message = json!({ "role": "assistant", "content": if self.text.is_empty() && has_calls { Value::Null } else { json!(self.text) } });
        if has_calls {
            message["tool_calls"] = Value::Array(self.calls.into_iter().map(Value::Object).collect());
        }
        let finish = self.finish_reason.unwrap_or_else(|| if has_calls { "tool_calls".into() } else { "stop".into() });
        json!({
            "object": "chat.completion",
            "model": model,
            "choices": [{ "index": 0, "message": message, "finish_reason": finish }],
            "usage": self.usage.unwrap_or(Value::Null),
        })
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
    fn tool_calling_requests_match_the_golden_fixture() {
        let text = include_str!("../tests/fixtures/chat_request_tools.json");
        let req: ChatRequest = serde_json::from_str(text).unwrap();
        assert_eq!(req.messages.len(), 4);
        assert_eq!(req.messages[2].tool_calls.as_ref().unwrap()[0]["function"]["name"], "get_weather");
        assert_eq!(req.messages[3].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(req.tools.as_ref().unwrap().len(), 1);
        let back = serde_json::to_value(&req).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(back, fixture);
    }

    #[test]
    fn streamed_tool_calls_are_put_back_together() {
        use serde_json::json;
        let mut b = CompletionBuilder::default();
        for c in [
            json!({"choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"Lagos\"}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}),
        ] {
            b.push(&c);
        }
        let done = b.finish("m");
        let msg = &done["choices"][0]["message"];
        assert_eq!(msg["content"], serde_json::Value::Null);
        assert_eq!(msg["tool_calls"][0]["id"], "call_1");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(msg["tool_calls"][0]["function"]["arguments"], "{\"city\":\"Lagos\"}");
        assert_eq!(done["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(done["usage"]["completion_tokens"], 5);
    }

    #[test]
    fn plain_text_completions_still_work() {
        use serde_json::json;
        let mut b = CompletionBuilder::default();
        b.push(&json!({"choices":[{"delta":{"content":"Hel"}}]}));
        b.push(&json!({"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}));
        let done = b.finish("m");
        assert_eq!(done["choices"][0]["message"]["content"], "Hello");
        assert!(done["choices"][0]["message"].get("tool_calls").is_none());
        assert_eq!(done["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn a_null_content_reads_as_empty_text() {
        let m: ChatMessage = serde_json::from_value(serde_json::json!({"role":"assistant","content":null,"tool_calls":[]})).unwrap();
        assert_eq!(m.content.text(), "");
        let m: ChatMessage = serde_json::from_value(serde_json::json!({"role":"assistant"})).unwrap();
        assert_eq!(m.content.text(), "");
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
