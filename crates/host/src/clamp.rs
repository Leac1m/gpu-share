//! Request clamping (`P2-T7`, spec §6.8): what the gatekeeper forwards to the engine.
//!
//! The forwarded body is built from scratch out of an allowlist, never by passing the client's
//! JSON through. `max_tokens` is capped at the owner's limit, sampling parameters are clamped
//! to their sane ranges, and anything llama-server understands but the owner didn't agree to
//! (slot ids, grammars, `n`, logprobs, `n_predict`, cache files) can't get through because
//! [`ChatRequest`] doesn't have a field for it.

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
        if !matches!(m.role.as_str(), "system" | "user" | "assistant") {
            return Err(ClampError::BadRole);
        }
        let text = m.content.text();
        total += text.len();
        messages.push(json!({ "role": m.role, "content": text }));
    }
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
        assert_eq!(
            engine_body(&req(json!({"model":"m","messages":[{"role":"tool","content":"x"}]})), LIMITS, "m"),
            Err(ClampError::BadRole)
        );
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
