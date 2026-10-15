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
use host::runtime::HostRuntime;
use host::world::{FileWorld, SystemWorld, World};
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

/// The engine manifest bundled by the release workflow (empty in development builds).
/// `GPU_SHARE_ENGINE_MANIFEST` points at another one, to try the first-run download locally.
fn engine_manifest(app: &tauri::AppHandle) -> host::engine_setup::Manifest {
    env_path("GPU_SHARE_ENGINE_MANIFEST")
        .or_else(|| app.path().resource_dir().ok().map(|d| d.join("engine-manifest.json")))
        .and_then(|p| host::engine_setup::Manifest::load(&p))
        .unwrap_or_default()
}

/// Flags that put the model where it should run. Windows runs the CUDA build; Linux runs the
/// Vulkan build, on the NVIDIA card when there is one, otherwise on the processor only
/// (`--device none`: integrated graphics are often slower than the processor, and their memory
/// isn't what the budget measures).
fn placement_args(card: bool) -> Vec<String> {
    if cfg!(windows) || card {
        vec!["-ngl".into(), "999".into()]
    } else {
        vec!["--device".into(), "none".into()]
    }
}

/// Where llama-server is, if it's on this computer already: an override, a copy bundled in the
/// app, one set up on an earlier run, or the dev build.
fn engine_binary(app: &tauri::AppHandle, data_dir: &Path, manifest: &host::engine_setup::Manifest) -> Option<PathBuf> {
    let exe = host::engine_setup::binary_name();
    let candidates = [
        env_path("GPU_SHARE_LLAMA_SERVER"),
        app.path().resource_dir().ok().map(|d| d.join("engine").join(exe)),
        (!manifest.tag.is_empty()).then(|| host::engine_setup::engine_dir(data_dir, &manifest.tag).join(exe)),
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
        let manifest = engine_manifest(app);
        // the real machine, or (for development) signals read from a JSON file
        let world: Arc<dyn World> = match env_path("GPU_SHARE_WORLD_FILE") {
            Some(path) => Arc::new(FileWorld { path }),
            None => Arc::new(SystemWorld::new()),
        };
        let card = world.card().map(|(name, vram_gb)| host::core::Card { name, vram_gb });
        let has_card = card.is_some();
        // Share mode needs an engine on this computer, or one to download: on Windows only with
        // an NVIDIA card (fetched at once), on Linux always (fetched on the first switch-on)
        let found = engine_binary(app, &data_dir, &manifest);
        let to_set_up = (found.is_none() && !manifest.files.is_empty() && (card.is_some() || !cfg!(windows)))
            .then(|| host::engine_setup::engine_dir(&data_dir, &manifest.tag));
        let engine = found.clone().or_else(|| to_set_up.as_ref().map(|d| d.join(host::engine_setup::binary_name())));
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
                    port,
                    card,
                    engine: EngineConfig {
                        binary,
                        extra_args: placement_args(has_card),
                        start_timeout: Duration::from_secs(180),
                        memory_cap: None,
                        env: vec![],
                    },
                    catalog: bundled_catalog(),
                    dev_models: cfg!(debug_assertions) || std::env::var("GPU_SHARE_DEV").is_ok(),
                    download_hosts: host::library::DEFAULT_HOSTS.iter().map(|h| h.to_string()).collect(),
                })?;
                {
                    // the published catalog (P6-T1, D3); offline starts use the cache, then the
                    // bundled copy. Release builds fetch it by default, development builds only
                    // with GPU_SHARE_CATALOG_URL (an empty value turns fetching off).
                    let core = core.clone();
                    let url = match std::env::var("GPU_SHARE_CATALOG_URL") {
                        Ok(u) if u.is_empty() => None,
                        Ok(u) => Some(u),
                        Err(_) => (!cfg!(debug_assertions)).then(|| host::library::DEFAULT_CATALOG_URL.to_string()),
                    };
                    let cache = data_dir.join("catalog-cache.json");
                    tauri::async_runtime::spawn(async move {
                        if url.is_some() {
                            let hosts = core.cfg.download_hosts.clone();
                            let list = host::library::load_catalog(bundled_catalog(), url.as_deref(), &cache, &hosts, env!("CARGO_PKG_VERSION")).await;
                            core.set_catalog(list);
                        }
                    });
                }
                if let Some(dir) = to_set_up {
                    // first use on this computer: fetch the engine once (P7-T3, P8-T1)
                    let when = if cfg!(windows) { host::engine_setup::When::Now } else { host::engine_setup::When::FirstShare };
                    tauri::async_runtime::spawn(host::engine_setup::drive(core.clone(), manifest, dir, when));
                }
                let rt = HostRuntime::new(core, SocketAddr::from(([0, 0, 0, 0], port)), world);
                tauri::async_runtime::spawn(rt.clone().run());
                tauri::async_runtime::spawn(host::announce::run(rt.core.clone()));
                Some(rt)
            }
        };
        Ok(Arc::new(Self { host, client, discovery }))
    }
}
