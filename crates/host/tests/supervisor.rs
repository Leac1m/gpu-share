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

/// Linux runs the engine below the owner's programs (the Windows build uses a Job Object).
#[cfg(target_os = "linux")]
#[tokio::test]
async fn on_linux_the_engine_runs_at_a_lower_priority() {
    let sup = supervisor(&[]);
    sup.ensure(&spec("a"), |_| {}).await.unwrap();
    let pid = sup.pid().await.unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // fields after the ")" that ends the command name; nice is field 19 overall
    let nice: i32 = stat.rsplit(')').next().unwrap().split_whitespace().nth(16).unwrap().parse().unwrap();
    assert!(nice >= host::engine::ENGINE_NICE, "nice {nice}");
    sup.stop().await;
}

#[tokio::test]
async fn a_port_taken_between_choosing_and_binding_is_retried_on_another() {
    // free_port() lets go of the port before the engine binds it, so something else can grab it
    let dir = tempfile::tempdir().unwrap();
    let flag = dir.path().join("taken");
    let sup = supervisor(&[("FAKE_PORT_TAKEN_ONCE", flag.to_str().unwrap())]);
    let h = sup.ensure(&spec("a"), |_| {}).await.unwrap();
    assert!(flag.exists(), "the first attempt hit the taken port");
    let ports: Vec<_> = sup.recent_log().iter().filter_map(|l| l.split("--port ").nth(1).map(|p| p.split(' ').next().unwrap().to_string())).collect();
    assert_eq!(ports.len(), 2, "{:?}", sup.recent_log());
    assert!(h.base_url.ends_with(&ports[1]), "the handle points at the second attempt's port");
    assert_eq!(sup.status().error, None);
    sup.stop().await;
}

#[tokio::test]
async fn an_engine_that_always_exits_fails_after_a_few_tries() {
    let sup = Supervisor::new(EngineConfig {
        binary: PathBuf::from("/bin/false"),
        extra_args: vec![],
        start_timeout: Duration::from_secs(10),
        memory_cap: None,
        env: vec![],
    });
    let started = std::time::Instant::now();
    assert!(matches!(sup.ensure(&spec("a"), |_| {}).await, Err(EngineError::Exited(_))));
    assert!(started.elapsed() < Duration::from_secs(3), "gives up quickly");
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

/// Stand-in for the app in the test below: starts an engine, prints its pid, waits to be killed.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn engine_parent_helper() {
    if std::env::var_os("GPU_SHARE_ENGINE_PARENT").is_none() {
        return; // only runs when the test below starts it
    }
    let sup = supervisor(&[]);
    sup.ensure(&spec("a"), |_| {}).await.unwrap();
    println!("ENGINE_PID={}", sup.pid().await.unwrap());
    tokio::time::sleep(Duration::from_secs(60)).await;
}

/// `P8-T5`: if the app is killed outright, the engine dies with it (`PR_SET_PDEATHSIG`) instead of
/// holding the memory with nobody to stop it.
#[cfg(target_os = "linux")]
#[test]
fn the_engine_dies_with_the_app_even_on_kill_9() {
    use std::io::BufRead;
    let mut app = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "engine_parent_helper", "--nocapture", "--test-threads=1"])
        .env("GPU_SHARE_ENGINE_PARENT", "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let out = std::io::BufReader::new(app.stdout.take().unwrap());
    let pid: u32 = out
        .lines()
        .map_while(Result::ok)
        .find_map(|l| l.split("ENGINE_PID=").nth(1).and_then(|p| p.trim().parse().ok()))
        .expect("the helper started an engine");
    assert!(alive(pid));
    app.kill().unwrap(); // SIGKILL: no cleanup code runs in the app
    app.wait().unwrap();
    let dead = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s.rsplit(')').next().unwrap().split_whitespace().next() == Some("Z"))
            .unwrap_or(true)
    };
    for _ in 0..50 {
        if dead() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the engine (pid {pid}) outlived the app");
}
