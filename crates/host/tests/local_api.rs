//! The local API (`P9-T2`, `P9-T3`): an OpenAI-compatible address on a friend's computer that
//! forwards to a paired host, tool calls included.

mod common;
use common::*;

use client::history::{History, KnownHost};
use client::local_api::LocalApi;
use client::runtime::ClientRuntime;
use serde_json::{json, Value};

const KEY: &str = "gs-test-key-0123456789abcdef0123456789abcdef";

struct Setup {
    host: TestHost,
    _api: LocalApi,
    base: String,
    http: reqwest::Client,
}

async fn setup(engine_env: &[(&str, &str)]) -> Setup {
    let host = start_host(engine_env).await;
    let id = netcore::Identity::generate().unwrap();
    host.core.store.pair(id.fingerprint(), "Friend PC", "tok", 1).unwrap();
    let history = History::in_memory().unwrap();
    history
        .save_host(&KnownHost { id: host.core.identity.fingerprint().into(), name: "Test host".into(), token: "tok".into(), address: host.addr.to_string() })
        .unwrap();
    let rt = ClientRuntime::new(id, history, "Friend PC".into());
    rt.refresh().await; // finds the host at its saved address and fetches its models
    let api = LocalApi::start(rt.clone(), 0, KEY.into()).await.unwrap();
    assert!(api.addr.ip().is_loopback());
    let base = format!("http://{}/v1", api.addr);
    Setup { host, _api: api, base, http: reqwest::Client::builder().no_proxy().build().unwrap() }
}

impl Setup {
    async fn post(&self, body: Value) -> reqwest::Response {
        self.http.post(format!("{}/chat/completions", self.base)).bearer_auth(KEY).json(&body).send().await.unwrap()
    }
}

#[tokio::test]
async fn only_this_computer_with_the_key_gets_in() {
    let s = setup(&[]).await;
    let models = format!("{}/models", s.base);
    assert_eq!(s.http.get(&models).send().await.unwrap().status(), 401);
    assert_eq!(s.http.get(&models).bearer_auth("gs-wrong").send().await.unwrap().status(), 401);
    // a web page reaching it through DNS rebinding sends its own name as Host
    let r = s.http.get(&models).bearer_auth(KEY).header("host", "evil.example:50125").send().await.unwrap();
    assert_eq!(r.status(), 403);
    for path in ["/v1/completions", "/v1/embeddings", "/api/pair", "/"] {
        let url = s.base.trim_end_matches("/v1").to_string() + path;
        assert_eq!(s.http.get(&url).bearer_auth(KEY).send().await.unwrap().status(), 404, "{path}");
    }
}

#[tokio::test]
async fn models_lists_what_the_paired_host_offers() {
    let s = setup(&[]).await;
    let v: Value = s.http.get(format!("{}/models", s.base)).bearer_auth(KEY).send().await.unwrap().json().await.unwrap();
    let ids: Vec<&str> = v["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["model-a", "model-b"]);
    assert_eq!(v["data"][0]["owned_by"], "Test host");
}

#[tokio::test]
async fn a_plain_completion_comes_back_in_the_openai_shape() {
    let s = setup(&[]).await;
    let v: Value = s.post(json!({"model":"model-a","messages":[{"role":"user","content":"hi there"}]})).await.json().await.unwrap();
    assert_eq!(v["object"], "chat.completion");
    assert_eq!(v["choices"][0]["message"]["content"].as_str().unwrap().trim(), "Echo: hi there");
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
    assert!(v["id"].as_str().unwrap().starts_with("chatcmpl-"));
    assert!(v["usage"]["completion_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn streaming_passes_openai_chunks_and_nothing_else() {
    let s = setup(&[]).await;
    let r = s.post(json!({"model":"model-a","stream":true,"messages":[{"role":"user","content":"stream me"}]})).await;
    assert_eq!(r.headers()["content-type"], "text/event-stream");
    let body = r.text().await.unwrap();
    assert!(!body.contains("event:"), "the host's queue messages stay inside GPU Share: {body}");
    let frames: Vec<&str> = body.split("\n\n").filter(|f| !f.is_empty()).collect();
    assert!(frames.iter().all(|f| f.starts_with("data: ")), "{body}");
    assert_eq!(*frames.last().unwrap(), "data: [DONE]");
    let text: String = frames
        .iter()
        .filter_map(|f| serde_json::from_str::<Value>(&f[6..]).ok())
        .filter_map(|v| v["choices"][0]["delta"]["content"].as_str().map(String::from))
        .collect();
    assert_eq!(text.trim(), "Echo: stream me");
}

fn weather_tool() -> Value {
    json!({"type":"function","function":{"name":"get_weather","description":"Weather for a city","parameters":{"type":"object","properties":{"city":{"type":"string"}}}}})
}

#[tokio::test]
async fn a_harness_can_call_tools_and_send_back_their_results() {
    let s = setup(&[]).await;
    let ask = json!({"role":"user","content":"TOOL: weather in Lagos?"});
    let v: Value = s.post(json!({"model":"model-a","tools":[weather_tool()],"tool_choice":"auto","messages":[ask]})).await.json().await.unwrap();
    let msg = &v["choices"][0]["message"];
    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(msg["content"], Value::Null);
    let call = &msg["tool_calls"][0];
    assert_eq!(call["function"]["name"], "get_weather");
    assert_eq!(serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap(), json!({"city":"Lagos"}));

    // the harness runs the tool itself, then sends the result back
    let v: Value = s
        .post(json!({"model":"model-a","tools":[weather_tool()],"messages":[
            ask, msg.clone(), {"role":"tool","tool_call_id":call["id"],"content":"31C"}
        ]}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["choices"][0]["message"]["content"].as_str().unwrap().trim(), "Echo: tool said 31C");
}

#[tokio::test]
async fn streamed_tool_calls_arrive_as_deltas() {
    let s = setup(&[]).await;
    let body = s
        .post(json!({"model":"model-a","stream":true,"tools":[weather_tool()],"messages":[{"role":"user","content":"TOOL please"}]}))
        .await
        .text()
        .await
        .unwrap();
    let args: String = body
        .split("\n\n")
        .filter_map(|f| f.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str().map(String::from))
        .collect();
    assert_eq!(args, r#"{"city":"Lagos"}"#);
}

#[tokio::test]
async fn unknown_models_and_paused_hosts_are_openai_errors() {
    let s = setup(&[]).await;
    let r = s.post(json!({"model":"gpt-9","messages":[{"role":"user","content":"x"}]})).await;
    assert_eq!(r.status(), 404);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "model_not_found");
    assert!(v["error"]["message"].as_str().unwrap().contains("model-a"), "says what is available");

    s.host.core.set_gate(host::core::Gate::Paused { owner: "x".into(), friend: "They're using their computer".into(), countdown: false });
    let r = s.post(json!({"model":"model-a","messages":[{"role":"user","content":"x"}]})).await;
    assert_eq!(r.status(), 503);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "host_paused");
    assert!(v["error"]["message"].as_str().unwrap().contains("using their computer"));
}

#[tokio::test]
async fn a_bad_tool_definition_is_refused_by_the_host_not_passed_on() {
    let s = setup(&[]).await;
    let bad = json!({"type":"function","function":{"name":"rm -rf"}});
    let r = s.post(json!({"model":"model-a","tools":[bad],"messages":[{"role":"user","content":"x"}]})).await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"]["message"].as_str().unwrap().contains("letters, digits"), "{v}");
}
