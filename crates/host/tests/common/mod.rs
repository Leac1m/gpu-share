//! A real gatekeeper on 127.0.0.1 over the fake engine, and clients to talk to it.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use client::api::HostClient;
use host::core::{HostConfig, HostCore};
use host::engine::EngineConfig;
use host::gatekeeper::Gatekeeper;
use netcore::tls::ServerPin;
use netcore::Identity;
use protocol::models::{CatalogEntry, ModelShape};

pub struct TestHost {
    pub core: Arc<HostCore>,
    pub gk: Option<Gatekeeper>,
    pub addr: SocketAddr,
    pub dir: tempfile::TempDir,
}

pub fn entry(id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        name: format!("Model {id}"),
        description: String::new(),
        file_url: format!("https://huggingface.co/test/test/resolve/main/{id}.gguf"),
        size_bytes: 1,
        sha256: "0".repeat(64),
        quantization: "Q4_K_M".into(),
        license: "Apache 2.0".into(),
        default_context: 4096,
        shape: Some(ModelShape { layers: 1, kv_heads: 1, head_dim: 1 }),
        tps_estimate: None,
        engine_args: vec![],
        dev_only: false,
        min_app_version: None,
    }
}

pub async fn start_host(engine_env: &[(&str, &str)]) -> TestHost {
    start_host_with(engine_env, PathBuf::from(env!("CARGO_BIN_EXE_fake-llama-server")), vec![entry("model-a"), entry("model-b")], vec![]).await
}

pub async fn start_host_with(
    engine_env: &[(&str, &str)],
    binary: PathBuf,
    catalog: Vec<CatalogEntry>,
    model_files: Vec<(String, PathBuf)>,
) -> TestHost {
    let dir = tempfile::tempdir().unwrap();
    let models_dir = dir.path().join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    for e in &catalog {
        let dst = models_dir.join(e.file_name());
        match model_files.iter().find(|(id, _)| *id == e.id) {
            Some((_, src)) => symlink_or_copy(src, &dst),
            None => std::fs::write(&dst, b"fake gguf").unwrap(),
        }
    }
    let core = HostCore::open(HostConfig {
        data_dir: dir.path().join("data"),
        models_dir,
        name: "Test host".into(),
        os: protocol::discovery::Os::Linux,
        app_version: "0.0.0-test".into(),
        port: protocol::GATEKEEPER_PORT,
        card: None,
        engine: EngineConfig {
            binary,
            extra_args: vec![],
            start_timeout: Duration::from_secs(60),
            memory_cap: None,
            env: engine_env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        },
        catalog,
        dev_models: true,
        download_hosts: vec!["huggingface.co".into(), "127.0.0.1".into()],
    })
    .unwrap();
    core.set_sharing(true);
    core.set_gate(host::core::Gate::Open);
    let gk = Gatekeeper::start(core.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = gk.addr;
    TestHost { core, gk: Some(gk), addr, dir }
}

fn symlink_or_copy(src: &std::path::Path, dst: &std::path::Path) {
    #[cfg(unix)]
    if std::os::unix::fs::symlink(src, dst).is_ok() {
        return;
    }
    std::fs::copy(src, dst).unwrap();
}

/// A client device. `paired` registers it with the host the way pairing would.
pub struct TestClient {
    pub id: Identity,
    pub api: HostClient,
}

impl TestHost {
    pub fn client(&self, paired: bool) -> TestClient {
        let id = Identity::generate().unwrap();
        let token = paired.then(|| {
            let t = format!("token-{}", &id.fingerprint()[..8]);
            self.core.store.pair(id.fingerprint(), &format!("Device {}", &id.fingerprint()[..4]), &t, 1).unwrap();
            t
        });
        let api = HostClient::new(&id, self.addr, ServerPin::Fingerprint(self.core.identity.fingerprint().into()), token).unwrap();
        TestClient { id, api }
    }

    pub fn client_with_token(&self, token: &str) -> TestClient {
        let id = Identity::generate().unwrap();
        let api = HostClient::new(&id, self.addr, ServerPin::Fingerprint(self.core.identity.fingerprint().into()), Some(token.into())).unwrap();
        TestClient { id, api }
    }
}

pub fn chat_req(model: &str, text: &str) -> protocol::api::ChatRequest {
    serde_json::from_value(serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": text}]
    }))
    .unwrap()
}

/// Raw HTTPS for paths the typed client doesn't know.
pub fn raw(host: &TestHost, id: &Identity) -> reqwest::Client {
    let tls = netcore::tls::client_config(id, ServerPin::Fingerprint(host.core.identity.fingerprint().into())).unwrap();
    reqwest::Client::builder().use_preconfigured_tls(tls).no_proxy().build().unwrap()
}
