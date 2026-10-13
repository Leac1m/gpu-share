//! Supervisor lifecycle against a fake llama-server (roadmap suite 9).

use std::path::PathBuf;
use std::time::Duration;

use host::engine::{EngineConfig, EngineError, ModelSpec, Supervisor};

fn supervisor(env: &[(&str, &str)]) -> Supervisor {
    Supervisor::new(EngineConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_fake-llama-server")),
        extra_args: vec![],
        start_timeout: Duration::from_secs(10),
        memory_cap: None,
        env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    })
}

fn spec(id: &str) -> ModelSpec {
    ModelSpec { id: id.into(), path: PathBuf::from(format!("/models/{id}.gguf")), ctx: 4096, args: vec![], size_bytes: 1 }
}

fn alive(pid: u32) -> bool {
    // A reaped process has no /proc entry; good enough for the Linux CI runner.
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[tokio::test]
async fn starts_on_localhost_with_a_fresh_key_and_reports_progress() {
    let sup = supervisor(&[]);
    let pcts = std::sync::Mutex::new(vec![]);
    let h = sup.ensure(&spec("a"), |p| pcts.lock().unwrap().push(p)).await.unwrap();
    assert!(h.base_url.starts_with("http://127.0.0.1:"));
    assert_eq!(h.api_key.len(), 64);
    assert!(!pcts.lock().unwrap().is_empty(), "loading reports progress");
    assert_eq!(sup.status().loaded.as_deref(), Some("a"));

    let r = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", h.base_url))
        .bearer_auth(&h.api_key)
        .json(&serde_json::json!({"messages":[{"role":"user","content":"hi"}]}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    let args = sup.recent_log().join("\n");
    assert!(args.contains("--host 127.0.0.1"), "{args}");
    assert!(args.contains("--parallel 1"), "{args}");
    assert!(!args.contains("slot-save-path"), "slot saving stays off");

    let key1 = h.api_key.clone();
    sup.stop().await;
    let h2 = sup.ensure(&spec("a"), |_| {}).await.unwrap();
    assert_ne!(key1, h2.api_key, "a new key for every start");
    sup.stop().await;
}

#[tokio::test]
async fn same_model_reuses_the_process_and_a_new_model_replaces_it() {
    let sup = supervisor(&[]);
    sup.ensure(&spec("a"), |_| {}).await.unwrap();
    let pid_a = sup.pid().await.unwrap();
    sup.ensure(&spec("a"), |_| {}).await.unwrap();
    assert_eq!(sup.pid().await, Some(pid_a));

    sup.ensure(&spec("b"), |_| {}).await.unwrap();
    let pid_b = sup.pid().await.unwrap();
    assert_ne!(pid_a, pid_b);
    assert!(!alive(pid_a), "switching models stops the old process (one process per model)");
    assert_eq!(sup.status().loaded.as_deref(), Some("b"));

    sup.stop().await;
    assert!(!alive(pid_b), "stop actually kills the process");
    assert_eq!(sup.status().loaded, None);
}

#[tokio::test]
async fn a_crash_is_noticed_and_the_next_request_restarts() {
    let sup = supervisor(&[]);
    let h = sup.ensure(&spec("a"), |_| {}).await.unwrap();
    let _ = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", h.base_url))
        .bearer_auth(&h.api_key)
        .json(&serde_json::json!({"messages":[{"role":"user","content":"please CRASH"}]}))
        .send()
        .await;
    let mut crashed = false;
    for _ in 0..50 {
        if sup.check().await {
            crashed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(crashed);
    assert!(sup.take_error().unwrap().contains("stopped unexpectedly"));
    assert!(sup.ensure(&spec("a"), |_| {}).await.is_ok());
    sup.stop().await;
}

#[tokio::test]
async fn a_start_that_never_finishes_loading_times_out() {
    let sup = Supervisor::new(EngineConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_fake-llama-server")),
        extra_args: vec![],
        start_timeout: Duration::from_millis(500),
        memory_cap: None,
        env: vec![("FAKE_LOAD_MS".into(), "60000".into())],
    });
    assert_eq!(sup.ensure(&spec("a"), |_| {}).await.unwrap_err(), EngineError::Timeout);
    assert!(sup.take_error().is_some(), "failures are surfaced for the UI");
}

#[tokio::test]
async fn a_missing_binary_is_reported_not_panicked() {
    let sup = Supervisor::new(EngineConfig {
        binary: PathBuf::from("/nonexistent/llama-server"),
        extra_args: vec![],
        start_timeout: Duration::from_secs(1),
        memory_cap: None,
        env: vec![],
    });
    assert!(matches!(sup.ensure(&spec("a"), |_| {}).await, Err(EngineError::Spawn(_))));
}
