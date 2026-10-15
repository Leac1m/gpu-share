//! Request clamping (`P2-T7`, spec §6.8): what the gatekeeper forwards to the engine.
//!
//! The forwarded body is built from scratch out of an allowlist, never by passing the client's
//! JSON through. `max_tokens` is capped at the owner's limit, sampling parameters are clamped
//! to their sane ranges, and anything llama-server understands but the owner didn't agree to
//! (slot ids, grammars, `n`, logprobs, `n_predict`, cache files) can't get through because
//! [`ChatRequest`] doesn't have a field for it.
//!
//! Tool calling (`P9-T3`) is let through in its standard OpenAI form, rebuilt field by field:
//! function tools with plain names, `tool_choice`, `tool` result messages and the assistant's
//! `tool_calls`. A tool call is only text the model writes; the host never runs anything, the
//! harness on the asking computer decides what to do with it.

use protocol::api::{ChatRequest, Stop};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Max reply length in tokens (spec §6.4).
    pub max_reply: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClampError {
    #[error("The request has no messages.")]
    NoMessages,
    #[error("Messages must come from the system, the user or the assistant.")]
    BadRole,
    #[error("The request is too large.")]
    TooLarge,
    #[error("{0}")]
    BadTool(String),
}

const MAX_TOOLS: usize = 64;
/// Serialized size limits: tools all together, and one `response_format` schema.
const MAX_TOOLS_BYTES: usize = 64 * 1024;
const MAX_SCHEMA_BYTES: usize = 16 * 1024;
const MAX_ID_CHARS: usize = 128;

/// OpenAI's rule for function names: letters, digits, `_` and `-`, at most 64.
fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn bad(msg: impl Into<String>) -> ClampError {
    ClampError::BadTool(msg.into())
}

fn function_name(f: &Value) -> Result<String, ClampError> {
    let name = f.get("name").and_then(Value::as_str).unwrap_or("");
    if !valid_name(name) {
        return Err(bad("A tool name may only use letters, digits, _ and -, up to 64 characters."));
    }
    Ok(name.to_string())
}

fn tool(t: &Value) -> Result<Value, ClampError> {
    if t.get("type").and_then(Value::as_str) != Some("function") {
        return Err(bad("Only function tools are supported."));
    }
    let f = t.get("function").filter(|f| f.is_object()).ok_or_else(|| bad("A tool needs a function."))?;
    let mut out = Map::new();
    out.insert("name".into(), json!(function_name(f)?));
    if let Some(d) = f.get("description") {
        out.insert("description".into(), json!(d.as_str().ok_or_else(|| bad("A tool's description must be text."))?));
    }
    if let Some(p) = f.get("parameters") {
        if !p.is_object() {
            return Err(bad("A tool's parameters must be a JSON schema object."));
        }
        out.insert("parameters".into(), p.clone());
    }
    Ok(json!({ "type": "function", "function": out }))
}

fn tool_call(c: &Value) -> Result<Value, ClampError> {
    let id = c.get("id").and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= MAX_ID_CHARS);
    let f = c.get("function").filter(|f| f.is_object());
    match (id, c.get("type").and_then(Value::as_str), f) {
        (Some(id), Some("function"), Some(f)) => {
            let args = f.get("arguments").and_then(Value::as_str).ok_or_else(|| bad("A tool call's arguments must be text."))?;
            Ok(json!({ "id": id, "type": "function", "function": { "name": function_name(f)?, "arguments": args } }))
        }
        _ => Err(bad("A tool call needs an id, type \"function\" and a function.")),
    }
}

fn tool_choice(c: &Value) -> Result<Value, ClampError> {
    match c {
        Value::String(s) if matches!(s.as_str(), "auto" | "none" | "required") => Ok(c.clone()),
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("function") => {
            let f = o.get("function").ok_or_else(|| bad("tool_choice needs a function name."))?;
            Ok(json!({ "type": "function", "function": { "name": function_name(f)? } }))
        }
        _ => Err(bad("tool_choice must be \"auto\", \"none\", \"required\" or a function.")),
    }
}

fn response_format(r: &Value) -> Result<Value, ClampError> {
    match r.get("type").and_then(Value::as_str) {
        Some(t @ ("text" | "json_object")) => Ok(json!({ "type": t })),
        Some("json_schema") => {
            let js = r.get("json_schema").filter(|j| j.is_object()).ok_or_else(|| bad("json_schema is missing."))?;
            let schema = js.get("schema").filter(|s| s.is_object()).ok_or_else(|| bad("json_schema needs a schema object."))?;
            if serde_json::to_string(schema).map(|t| t.len()).unwrap_or(usize::MAX) > MAX_SCHEMA_BYTES {
                return Err(bad("The JSON schema is too large."));
            }
            let mut out = Map::new();
            if let Some(n) = js.get("name").and_then(Value::as_str).filter(|n| valid_name(n)) {
                out.insert("name".into(), json!(n));
            }
            out.insert("schema".into(), schema.clone());
            if let Some(strict) = js.get("strict").and_then(Value::as_bool) {
                out.insert("strict".into(), json!(strict));
            }
            Ok(json!({ "type": "json_schema", "json_schema": out }))
        }
        _ => Err(bad("response_format must be text, json_object or json_schema.")),
    }
}

/// Rough cap on the text forwarded. The engine's context (`-c`) is the real limit; this only
/// stops a single request from making the host chew through megabytes of tokenizing.
pub const MAX_PROMPT_CHARS: usize = 200_000;
const MAX_STOP_SEQUENCES: usize = 4;
const MAX_STOP_CHARS: usize = 64;

/// Builds the body sent to llama-server's `/v1/chat/completions`. Always streams, and always
/// asks for the usage chunk so the host can count tokens.
pub fn engine_body(req: &ChatRequest, limits: Limits, model_alias: &str) -> Result<Value, ClampError> {
    if req.messages.is_empty() {
        return Err(ClampError::NoMessages);
    }
    let mut total = 0usize;
    let mut messages = Vec::with_capacity(req.messages.len());
    for m in &req.messages {
        let role = match m.role.as_str() {
            "system" | "developer" => "system",
            "user" => "user",
            "assistant" => "assistant",
            "tool" => "tool",
            _ => return Err(ClampError::BadRole),
        };
        let text = m.content.text();
        total += text.len();
        let mut out = json!({ "role": role, "content": text });
        if role == "tool" {
            let id = m.tool_call_id.as_deref().filter(|id| !id.is_empty() && id.len() <= MAX_ID_CHARS).ok_or(ClampError::BadRole)?;
            out["tool_call_id"] = json!(id);
        }
        if let (Some(calls), "assistant") = (&m.tool_calls, role) {
            let calls = calls.iter().map(tool_call).collect::<Result<Vec<_>, _>>()?;
            total += calls.iter().map(|c| c.to_string().len()).sum::<usize>();
            if !calls.is_empty() {
                out["tool_calls"] = json!(calls);
            }
        }
        messages.push(out);
    }
    let tools = match &req.tools {
        Some(list) if list.len() > MAX_TOOLS => return Err(bad(format!("At most {MAX_TOOLS} tools."))),
        Some(list) => {
            let tools = list.iter().map(tool).collect::<Result<Vec<_>, _>>()?;
            let size = Value::Array(tools.clone()).to_string().len();
            if size > MAX_TOOLS_BYTES {
                return Err(bad("The tool definitions are too large."));
            }
            total += size;
            Some(tools)
        }
        None => None,
    };
    if total > MAX_PROMPT_CHARS {
        return Err(ClampError::TooLarge);
    }

    let asked = req.max_tokens.or(req.max_completion_tokens).unwrap_or(limits.max_reply);
    let max_tokens = asked.clamp(1, limits.max_reply.max(1));

    let mut body = Map::new();
    body.insert("model".into(), json!(model_alias));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({ "include_usage": true }));
    body.insert("max_tokens".into(), json!(max_tokens));
    let mut put = |k: &str, v: Option<f64>, lo: f64, hi: f64| {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            body.insert(k.into(), json!(v.clamp(lo, hi)));
        }
    };
    put("temperature", req.temperature, 0.0, 2.0);
    put("top_p", req.top_p, 0.0, 1.0);
    put("presence_penalty", req.presence_penalty, -2.0, 2.0);
    put("frequency_penalty", req.frequency_penalty, -2.0, 2.0);
    if let Some(tools) = tools.filter(|t| !t.is_empty()) {
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(c) = &req.tool_choice {
        body.insert("tool_choice".into(), tool_choice(c)?);
    }
    if let Some(p) = req.parallel_tool_calls {
        body.insert("parallel_tool_calls".into(), json!(p));
    }
    if let Some(r) = &req.response_format {
        body.insert("response_format".into(), response_format(r)?);
    }
    if let Some(seed) = req.seed {
        body.insert("seed".into(), json!(seed));
    }
    if let Some(stop) = &req.stop {
        let list = match stop {
            Stop::One(s) => vec![s.clone()],
            Stop::Many(v) => v.clone(),
        };
        let list: Vec<String> = list
            .into_iter()
            .filter(|s| !s.is_empty())
            .take(MAX_STOP_SEQUENCES)
            .map(|s| s.chars().take(MAX_STOP_CHARS).collect())
            .collect();
        if !list.is_empty() {
            body.insert("stop".into(), json!(list));
        }
    }
    Ok(Value::Object(body))
}

/// Every key [`engine_body`] can emit. Anything else reaching the engine is a bug.
pub const FORWARDED_KEYS: &[&str] = &[
    "model",
    "messages",
    "stream",
    "stream_options",
    "max_tokens",
    "temperature",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "seed",
    "stop",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "response_format",
];

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn req(v: Value) -> ChatRequest {
        serde_json::from_value(v).unwrap()
    }

    const LIMITS: Limits = Limits { max_reply: 1024 };

    #[test]
    fn a_missing_max_tokens_gets_the_owners_limit() {
        let b = engine_body(&req(json!({"model":"m","messages":[{"role":"user","content":"hi"}]})), LIMITS, "m").unwrap();
        assert_eq!(b["max_tokens"], 1024);
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);
    }

    #[test]
    fn a_huge_max_tokens_is_capped_and_a_small_one_kept() {
        let big = engine_body(&req(json!({"model":"m","messages":[{"role":"user","content":"hi"}],"max_tokens":999999})), LIMITS, "m").unwrap();
        assert_eq!(big["max_tokens"], 1024);
        let small = engine_body(&req(json!({"model":"m","messages":[{"role":"user","content":"hi"}],"max_completion_tokens":50})), LIMITS, "m").unwrap();
        assert_eq!(small["max_tokens"], 50);
    }

    #[test]
    fn engine_specific_parameters_never_reach_the_engine() {
        let r = req(json!({
            "model":"m","messages":[{"role":"user","content":"hi"}],
            "id_slot": 3, "n_predict": -1, "n": 4, "grammar": "x", "logprobs": true,
            "cache_prompt": true, "slot_save_path": "/tmp/x", "n_probs": 10, "stream": false
        }));
        let b = engine_body(&r, LIMITS, "m").unwrap();
        for k in b.as_object().unwrap().keys() {
            assert!(FORWARDED_KEYS.contains(&k.as_str()), "{k} must not be forwarded");
        }
        assert_eq!(b["stream"], true, "the host always streams from the engine");
    }

    #[test]
    fn sampling_parameters_are_clamped() {
        let b = engine_body(&req(json!({"model":"m","messages":[{"role":"user","content":"x"}],"temperature":50,"top_p":-3})), LIMITS, "m").unwrap();
        assert_eq!(b["temperature"], 2.0);
        assert_eq!(b["top_p"], 0.0);
    }

    #[test]
    fn the_model_field_is_the_hosts_alias_not_the_clients_string() {
        let b = engine_body(&req(json!({"model":"../../etc/passwd","messages":[{"role":"user","content":"x"}]})), LIMITS, "qwen3-8b").unwrap();
        assert_eq!(b["model"], "qwen3-8b");
    }

    #[test]
    fn odd_roles_and_empty_requests_are_refused() {
        assert_eq!(engine_body(&req(json!({"model":"m","messages":[]})), LIMITS, "m"), Err(ClampError::NoMessages));
        // a tool result has to say which call it answers
        assert_eq!(
            engine_body(&req(json!({"model":"m","messages":[{"role":"tool","content":"x"}]})), LIMITS, "m"),
            Err(ClampError::BadRole)
        );
        assert_eq!(
            engine_body(&req(json!({"model":"m","messages":[{"role":"function","content":"x"}]})), LIMITS, "m"),
            Err(ClampError::BadRole)
        );
    }

    // ---------------------------------------------------------------- tool calling (P9-T3)

    fn tool(name: &str) -> Value {
        json!({"type":"function","function":{"name":name,"description":"d","parameters":{"type":"object","properties":{}}}})
    }

    fn with(extra: Value) -> Result<Value, ClampError> {
        let mut v = json!({"model":"m","messages":[{"role":"user","content":"hi"}]});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        engine_body(&req(v), LIMITS, "m")
    }

    #[test]
    fn the_golden_tool_calling_request_goes_through() {
        let r: ChatRequest = serde_json::from_str(include_str!("../../protocol/tests/fixtures/chat_request_tools.json")).unwrap();
        let b = engine_body(&r, LIMITS, "m").unwrap();
        assert_eq!(b["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(b["tool_choice"], "auto");
        assert_eq!(b["parallel_tool_calls"], false);
        assert_eq!(b["messages"][2]["tool_calls"][0]["function"]["arguments"], "{\"city\":\"Lagos\"}");
        assert_eq!(b["messages"][3]["role"], "tool");
        assert_eq!(b["messages"][3]["tool_call_id"], "call_1");
    }

    #[test]
    fn tools_are_rebuilt_with_only_the_known_fields() {
        let mut t = tool("search");
        t["function"]["x-run-on-host"] = json!("rm -rf /");
        t["cache_control"] = json!({"type":"ephemeral"});
        let b = with(json!({"tools":[t]})).unwrap();
        assert_eq!(b["tools"][0], tool("search"));
    }

    #[test]
    fn only_function_tools_with_plain_names_are_accepted() {
        assert!(with(json!({"tools":[tool("get_weather"), tool("read-file_2")]})).is_ok());
        for bad in [
            json!({"type":"code_interpreter"}),
            json!({"type":"function","function":{"name":"has space"}}),
            json!({"type":"function","function":{"name":""}}),
            json!({"type":"function","function":{"name":"x".repeat(65)}}),
            json!({"type":"function","function":{"name":"f","parameters":"not an object"}}),
            json!({"type":"function"}),
        ] {
            assert!(matches!(with(json!({"tools":[bad.clone()]})), Err(ClampError::BadTool(_))), "{bad}");
        }
        let many: Vec<Value> = (0..65).map(|i| tool(&format!("t{i}"))).collect();
        assert!(matches!(with(json!({"tools":many})), Err(ClampError::BadTool(_))));
        let mut huge = tool("big");
        huge["function"]["description"] = json!("x".repeat(70_000));
        assert!(with(json!({"tools":[huge]})).is_err());
    }

    #[test]
    fn tool_choice_takes_the_standard_forms_only() {
        for ok in [json!("auto"), json!("none"), json!("required"), json!({"type":"function","function":{"name":"search"}})] {
            assert!(with(json!({"tools":[tool("search")],"tool_choice":ok.clone()})).is_ok(), "{ok}");
        }
        for bad in [json!("always"), json!({"type":"grammar","function":{"name":"x"}}), json!({"type":"function"}), json!(3)] {
            assert!(matches!(with(json!({"tools":[tool("search")],"tool_choice":bad.clone()})), Err(ClampError::BadTool(_))), "{bad}");
        }
    }

    #[test]
    fn assistant_tool_calls_are_checked_and_rebuilt() {
        let call = json!({"id":"c1","type":"function","function":{"name":"search","arguments":"{}"},"extra":"dropped"});
        let b = with(json!({"messages":[
            {"role":"user","content":"x"},
            {"role":"assistant","content":null,"tool_calls":[call]},
            {"role":"tool","tool_call_id":"c1","content":"result"}
        ]})).unwrap();
        assert_eq!(b["messages"][1]["tool_calls"][0], json!({"id":"c1","type":"function","function":{"name":"search","arguments":"{}"}}));
        assert_eq!(b["messages"][1]["content"], "");
        let bad = json!({"id":"c1","type":"function","function":{"name":"search","arguments":{"not":"a string"}}});
        assert!(with(json!({"messages":[{"role":"assistant","content":"","tool_calls":[bad]}]})).is_err());
    }

    #[test]
    fn the_developer_role_is_read_as_system() {
        let b = with(json!({"messages":[{"role":"developer","content":"be brief"},{"role":"user","content":"x"}]})).unwrap();
        assert_eq!(b["messages"][0]["role"], "system");
    }

    #[test]
    fn response_format_takes_text_json_or_a_small_schema() {
        assert_eq!(with(json!({"response_format":{"type":"json_object"}})).unwrap()["response_format"], json!({"type":"json_object"}));
        let schema = json!({"type":"json_schema","json_schema":{"name":"r","schema":{"type":"object"},"strict":true}});
        assert_eq!(with(json!({"response_format":schema.clone()})).unwrap()["response_format"], schema);
        assert!(with(json!({"response_format":{"type":"grammar","grammar":"root ::= x"}})).is_err());
        let big = json!({"type":"json_schema","json_schema":{"schema":{"description":"x".repeat(20_000)}}});
        assert!(with(json!({"response_format":big})).is_err());
    }

    #[test]
    fn stop_sequences_are_bounded() {
        let many: Vec<String> = (0..20).map(|i| "s".repeat(200 + i)).collect();
        let b = engine_body(&req(json!({"model":"m","messages":[{"role":"user","content":"x"}],"stop":many})), LIMITS, "m").unwrap();
        let stop = b["stop"].as_array().unwrap();
        assert_eq!(stop.len(), 4);
        assert!(stop.iter().all(|s| s.as_str().unwrap().len() == 64));
    }

    proptest! {
        /// Roadmap suite 6: max_tokens in the forwarded request never exceeds the owner's
        /// limit, whatever the client sends.
        #[test]
        fn forwarded_max_tokens_never_exceeds_the_limit(
            limit in 1u32..10_000,
            max_tokens in proptest::option::of(any::<u32>()),
            max_completion in proptest::option::of(any::<u32>()),
        ) {
            let mut v = json!({"model":"m","messages":[{"role":"user","content":"hi"}]});
            if let Some(t) = max_tokens { v["max_tokens"] = json!(t); }
            if let Some(t) = max_completion { v["max_completion_tokens"] = json!(t); }
            let b = engine_body(&req(v), Limits { max_reply: limit }, "m").unwrap();
            let got = b["max_tokens"].as_u64().unwrap();
            prop_assert!(got >= 1 && got <= limit as u64);
        }
    }
}
