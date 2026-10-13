//! A stand-in for `llama-server` in tests (roadmap suite 9). Speaks just enough of its HTTP API:
//! `/health` (503 while "loading"), and a streaming `/v1/chat/completions` that echoes the last
//! user message word by word and ends with a usage chunk.
//!
//! Behaviour knobs, via environment variables:
//! * `FAKE_LOAD_MS`  — how long `/health` reports loading (default 300)
//! * `FAKE_TOKEN_MS` — delay between words (default 20)
//! * a user message containing `CRASH` makes the process exit with status 3
//! * a missing or wrong `--api-key` bearer gets 401, like the real server

use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream;
use serde_json::{json, Value};

#[derive(Clone)]
struct Cfg {
    key: String,
    alias: String,
    ready_at: Instant,
    token_ms: u64,
}

fn arg(args: &[String], names: &[&str]) -> Option<String> {
    args.iter().position(|a| names.contains(&a.as_str())).and_then(|i| args.get(i + 1)).cloned()
}

fn env_ms(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = arg(&args, &["--port"]).and_then(|p| p.parse().ok()).expect("--port");
    let host = arg(&args, &["--host"]).unwrap_or_else(|| "127.0.0.1".into());
    let cfg = Cfg {
        key: arg(&args, &["--api-key"]).unwrap_or_default(),
        alias: arg(&args, &["--alias", "-a"]).unwrap_or_else(|| "fake".into()),
        ready_at: Instant::now() + Duration::from_millis(env_ms("FAKE_LOAD_MS", 300)),
        token_ms: env_ms("FAKE_TOKEN_MS", 20),
    };
    println!("fake llama-server on {host}:{port}, args: {}", args[1..].join(" "));
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat))
        .with_state(cfg);
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health(State(cfg): State<Cfg>) -> Response {
    if Instant::now() < cfg.ready_at {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": {"message": "Loading model"}}))).into_response()
    } else {
        Json(json!({"status": "ok"})).into_response()
    }
}

async fn chat(State(cfg): State<Cfg>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    if auth != format!("Bearer {}", cfg.key) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": {"message": "Invalid API Key"}}))).into_response();
    }
    let last = body["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|m| m["role"] == "user"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_string();
    if last.contains("CRASH") {
        std::process::exit(3);
    }
    let max = body["max_tokens"].as_u64().unwrap_or(u64::MAX) as usize;
    let words: Vec<String> = format!("Echo: {last}").split(' ').take(max).map(|w| format!("{w} ")).collect();
    let prompt_tokens = body["messages"].as_array().map(|m| m.len() * 4).unwrap_or(0);
    let n = words.len();
    let alias = cfg.alias.clone();
    let delay = Duration::from_millis(cfg.token_ms);
    let chunks = words
        .into_iter()
        .map(move |w| json!({"object": "chat.completion.chunk", "model": alias, "choices": [{"index": 0, "delta": {"content": w}}]}))
        .chain(std::iter::once(json!({
            "object": "chat.completion.chunk", "model": cfg.alias, "choices": [],
            "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": n, "total_tokens": prompt_tokens + n}
        })));
    let s = stream::unfold((chunks, false), move |(mut it, done)| async move {
        if done {
            return None;
        }
        tokio::time::sleep(delay).await;
        match it.next() {
            Some(c) => Some((Ok::<_, Infallible>(format!("data: {c}\n\n")), (it, false))),
            None => Some((Ok("data: [DONE]\n\n".to_string()), (it, true))),
        }
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(s))
        .unwrap()
}
