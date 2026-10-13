//! The engine supervisor (`P2-T2`, `P2-T3`, spec §6.2).
//!
//! * One `llama-server` process per model, never router mode (roadmap `P2-T2` note): switching
//!   models stops one process and starts another, and [`Supervisor::stop`] returns VRAM use to
//!   zero.
//! * The engine listens on `127.0.0.1` only, on a free port, with a random API key generated
//!   for each start, `--parallel 1`, and no `--slot-save-path` (it writes files).
//! * A crash is noticed by [`Supervisor::check`] and reported; the next request starts a fresh
//!   process.
//! * On Windows the process goes into a Job Object (see [`crate::jobobject`]).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::RngCore;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// How to run `llama-server` on this machine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub binary: PathBuf,
    /// Arguments added to every start, e.g. `-ngl 999` to put every layer on the GPU.
    pub extra_args: Vec<String>,
    /// Give up on a start after this long.
    pub start_timeout: Duration,
    /// Memory cap for the Job Object on Windows, in bytes. `None` = no cap.
    pub memory_cap: Option<u64>,
    /// Extra environment for the engine process.
    pub env: Vec<(String, String)>,
}

/// What to load.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    pub id: String,
    pub path: PathBuf,
    /// Context size in tokens (`-c`).
    pub ctx: u32,
    pub args: Vec<String>,
    /// Used to estimate load progress.
    pub size_bytes: u64,
}

/// A running engine, as the gatekeeper needs it.
#[derive(Debug, Clone)]
pub struct EngineHandle {
    pub base_url: String,
    pub api_key: String,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("the engine could not be started: {0}")]
    Spawn(String),
    #[error("the engine stopped while loading the model: {0}")]
    Exited(String),
    #[error("the engine took too long to load the model")]
    Timeout,
    #[error("loading was cancelled")]
    Cancelled,
}

/// What the Share screen shows about the engine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineStatus {
    pub loaded: Option<String>,
    pub loading: Option<String>,
    pub load_pct: u8,
    /// The last failure, for a toast; cleared when read by [`Supervisor::take_error`].
    pub error: Option<String>,
}

struct Running {
    child: Child,
    spec: ModelSpec,
    handle: EngineHandle,
    #[cfg(windows)]
    _job: crate::jobobject::Job,
}

#[derive(Default)]
struct Shared {
    status: EngineStatus,
    log: VecDeque<String>,
}

pub struct Supervisor {
    cfg: EngineConfig,
    running: tokio::sync::Mutex<Option<Running>>,
    shared: Arc<Mutex<Shared>>,
    http: reqwest::Client,
}

/// llama-server reads models at roughly this speed off an SSD; only used for the progress bar.
const LOAD_BYTES_PER_SEC: f64 = 2.5e9;
const LOG_LINES: usize = 40;

impl Supervisor {
    pub fn new(cfg: EngineConfig) -> Self {
        Self {
            cfg,
            running: tokio::sync::Mutex::new(None),
            shared: Arc::new(Mutex::new(Shared::default())),
            http: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap(),
        }
    }

    pub fn status(&self) -> EngineStatus {
        self.shared.lock().unwrap().status.clone()
    }

    pub fn take_error(&self) -> Option<String> {
        self.shared.lock().unwrap().status.error.take()
    }

    /// The last lines the engine printed, for diagnosing a failed start. Memory only.
    pub fn recent_log(&self) -> Vec<String> {
        self.shared.lock().unwrap().log.iter().cloned().collect()
    }

    /// Returns a handle to an engine running `spec`, starting one (and stopping any other
    /// model) if needed. `progress` gets an estimated 0–99 while loading.
    pub async fn ensure(&self, spec: &ModelSpec, progress: impl Fn(u8)) -> Result<EngineHandle, EngineError> {
        let mut running = self.running.lock().await;
        if let Some(r) = running.as_mut() {
            let alive = matches!(r.child.try_wait(), Ok(None));
            if alive && r.spec == *spec {
                return Ok(r.handle.clone());
            }
        }
        if let Some(r) = running.take() {
            Self::kill(r).await;
        }
        {
            let mut sh = self.shared.lock().unwrap();
            sh.status.loaded = None;
            sh.status.loading = Some(spec.id.clone());
            sh.status.load_pct = 0;
        }
        let result = self.start(spec, &progress).await;
        let mut sh = self.shared.lock().unwrap();
        sh.status.loading = None;
        sh.status.load_pct = 0;
        match result {
            Ok(r) => {
                sh.status.loaded = Some(spec.id.clone());
                let h = r.handle.clone();
                *running = Some(r);
                Ok(h)
            }
            Err(e) => {
                sh.status.error = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// Starts the engine, trying again on a fresh port if it exits before it's ready:
    /// [`free_port`] has to let go of the port before the engine binds it, so another program can
    /// take it in between. An engine that keeps exiting (a broken model file) fails after
    /// [`START_ATTEMPTS`] quick tries.
    async fn start(&self, spec: &ModelSpec, progress: &impl Fn(u8)) -> Result<Running, EngineError> {
        let mut attempt = 1;
        loop {
            match self.start_once(spec, progress).await {
                Err(EngineError::Exited(status)) if attempt < START_ATTEMPTS => {
                    log::warn!("engine exited while starting ({status}), trying again on another port");
                    attempt += 1;
                }
                r => return r,
            }
        }
    }

    async fn start_once(&self, spec: &ModelSpec, progress: &impl Fn(u8)) -> Result<Running, EngineError> {
        let port = free_port().map_err(|e| EngineError::Spawn(e.to_string()))?;
        let api_key = random_key();
        let mut cmd = Command::new(&self.cfg.binary);
        cmd.arg("-m")
            .arg(&spec.path)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["--api-key", &api_key])
            .args(["--parallel", "1"])
            .args(["--ctx-size", &spec.ctx.to_string()])
            .args(["--alias", &spec.id])
            .args(["--no-slots", "--offline"])
            .args(&self.cfg.extra_args)
            .args(&spec.args)
            .envs(self.cfg.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
            cmd.creation_flags(0x0800_0000 | 0x0000_4000);
        }
        let mut child = cmd.spawn().map_err(|e| EngineError::Spawn(e.to_string()))?;
        #[cfg(windows)]
        let job = crate::jobobject::Job::for_engine(&child, self.cfg.memory_cap).map_err(EngineError::Spawn)?;
        self.capture_output(&mut child);

        let base_url = format!("http://127.0.0.1:{port}");
        let started = Instant::now();
        let expected = Duration::from_secs_f64(1.0 + spec.size_bytes as f64 / LOAD_BYTES_PER_SEC);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                // Only the exit status: the engine's own output stays in memory (`recent_log`),
                // because errors get logged to disk and output can echo prompt text.
                return Err(EngineError::Exited(status.to_string()));
            }
            if started.elapsed() > self.cfg.start_timeout {
                let _ = child.kill().await;
                return Err(EngineError::Timeout);
            }
            if let Ok(r) = self.http.get(format!("{base_url}/health")).send().await {
                if r.status().is_success() {
                    break;
                }
            }
            let pct = (started.elapsed().as_secs_f64() / expected.as_secs_f64() * 100.0).min(99.0) as u8;
            self.shared.lock().unwrap().status.load_pct = pct;
            progress(pct);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(Running {
            child,
            spec: spec.clone(),
            handle: EngineHandle { base_url, api_key, model_id: spec.id.clone() },
            #[cfg(windows)]
            _job: job,
        })
    }

    /// Keeps the last lines of the engine's output in memory (never on disk: with verbose
    /// logging llama-server can print prompts).
    fn capture_output(&self, child: &mut Child) {
        let out = child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
        let err = child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
        for stream in [out, err].into_iter().flatten() {
            let shared = self.shared.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut sh = shared.lock().unwrap();
                    if sh.log.len() == LOG_LINES {
                        sh.log.pop_front();
                    }
                    sh.log.push_back(line);
                }
            });
        }
    }

    /// Stops the engine, freeing its memory. Safe to call when nothing runs.
    pub async fn stop(&self) {
        let r = self.running.lock().await.take();
        if let Some(r) = r {
            Self::kill(r).await;
        }
        let mut sh = self.shared.lock().unwrap();
        sh.status.loaded = None;
    }

    async fn kill(mut r: Running) {
        let _ = r.child.kill().await;
        let _ = r.child.wait().await;
    }

    /// Notices an engine that died on its own. Returns true if it had crashed. Called by the
    /// policy loop every couple of seconds.
    pub async fn check(&self) -> bool {
        let Ok(mut running) = self.running.try_lock() else {
            return false; // a start or a request holds it; check again next tick
        };
        let crashed = match running.as_mut() {
            Some(r) => match r.child.try_wait() {
                Ok(Some(status)) => Some(status.to_string()),
                _ => None,
            },
            None => None,
        };
        if let Some(status) = crashed {
            *running = None;
            let mut sh = self.shared.lock().unwrap();
            sh.status.loaded = None;
            sh.status.error = Some(format!("The engine stopped unexpectedly ({status}). It will restart with the next question."));
            return true;
        }
        false
    }

    /// The process id of the running engine, for tests.
    pub async fn pid(&self) -> Option<u32> {
        self.running.lock().await.as_ref().and_then(|r| r.child.id())
    }
}

const START_ATTEMPTS: u32 = 3;

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn random_key() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}
