//! Startup: where data lives, which engine binary to use, and the two runtimes.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use client::discovery::Discovery;
use client::history::History;
use client::runtime::ClientRuntime;
use host::core::{bundled_catalog, HostConfig, HostCore};
use host::engine::EngineConfig;
use host::runtime::{HostRuntime, SwitchOnly};
use protocol::discovery::{Announcement, Mode, Os};
use tauri::Manager;

pub struct AppState {
    /// `None` on machines that can't share (no engine available).
    pub host: Option<Arc<HostRuntime>>,
    pub client: Arc<ClientRuntime>,
    pub discovery: Arc<Discovery>,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Developer checkout root (debug builds only), for `.dev/` from `scripts/dev-engine.sh`.
fn dev_root() -> Option<PathBuf> {
    cfg!(debug_assertions).then(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn engine_binary(app: &tauri::AppHandle) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let candidates = [
        env_path("GPU_SHARE_LLAMA_SERVER"),
        app.path().resource_dir().ok().map(|d| d.join("engine").join(exe)),
        dev_root().map(|r| r.join(".dev").join(exe)),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// The computer's name, as friends will see it.
fn device_name() -> String {
    let raw = std::env::var("GPU_SHARE_NAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .unwrap_or_else(|_| "This computer".into());
    let name: String = raw.trim().chars().filter(|c| !c.is_control()).take(48).collect();
    if name.is_empty() {
        "This computer".into()
    } else {
        name
    }
}

/// Debug builds pick up the dev model from `.dev/models` so the app can chat with no download.
fn link_dev_model(models_dir: &Path) {
    let Some(root) = dev_root() else { return };
    let src = root.join(".dev/models/SmolLM2-135M-Instruct-Q8_0.gguf");
    let dst = models_dir.join("smollm2-135m-q8.gguf");
    if src.is_file() && !dst.exists() {
        let _ = std::fs::create_dir_all(models_dir);
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(&src, &dst);
        #[cfg(not(unix))]
        let _ = std::fs::copy(&src, &dst);
    }
}

impl AppState {
    pub fn init(app: &tauri::AppHandle) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let data_dir = env_path("GPU_SHARE_DATA_DIR").unwrap_or(app.path().app_data_dir()?);
        std::fs::create_dir_all(&data_dir)?;
        let name = device_name();
        let identity = netcore::Identity::load_or_create(&data_dir)?;
        let os = if cfg!(windows) { Os::Windows } else { Os::Linux };
        let engine = engine_binary(app);
        let me = Announcement {
            protocol: protocol::PROTOCOL_VERSION.into(),
            id: identity.fingerprint().into(),
            name: name.clone(),
            os,
            app_version: env!("CARGO_PKG_VERSION").into(),
            modes: if engine.is_some() { vec![Mode::Share, Mode::Use] } else { vec![Mode::Use] },
            status: None,
            port: protocol::GATEKEEPER_PORT,
            announce: true,
            reason: None,
            gpu: None,
        };

        let client = ClientRuntime::new(identity, History::open(&data_dir.join("client.db"))?, name.clone());
        let discovery = Discovery::new(client.clone(), me);
        tauri::async_runtime::spawn(discovery.clone().run());
        {
            let client = client.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    client.refresh().await;
                    client.expire(Duration::from_secs(protocol::discovery::HEARTBEAT_SECS * protocol::discovery::MISSED_HEARTBEATS));
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
        }

        let host = match engine {
            None => {
                log::info!("no llama-server found; Share mode is unavailable on this computer");
                None
            }
            Some(binary) => {
                let models_dir = env_path("GPU_SHARE_MODELS_DIR").unwrap_or(host::core::default_models_dir(&data_dir));
                link_dev_model(&models_dir);
                let port: u16 = std::env::var("GPU_SHARE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(protocol::GATEKEEPER_PORT);
                let core = HostCore::open(HostConfig {
                    data_dir: data_dir.clone(),
                    models_dir,
                    name,
                    os,
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    card: None,
                    engine: EngineConfig {
                        binary,
                        // the Windows host runs the CUDA build: every layer on the GPU
                        extra_args: if cfg!(windows) { vec!["-ngl".into(), "999".into()] } else { vec![] },
                        start_timeout: Duration::from_secs(180),
                        memory_cap: None,
                        env: vec![],
                    },
                    catalog: bundled_catalog(),
                    dev_models: cfg!(debug_assertions) || std::env::var("GPU_SHARE_DEV").is_ok(),
                })?;
                let rt = HostRuntime::new(core, SocketAddr::from(([0, 0, 0, 0], port)), Box::new(SwitchOnly));
                tauri::async_runtime::spawn(rt.clone().run());
                tauri::async_runtime::spawn(host::announce::run(rt.core.clone()));
                Some(rt)
            }
        };
        Ok(Arc::new(Self { host, client, discovery }))
    }
}
