//! `HostCore`: the state one Share-mode install keeps while running. The gatekeeper, the
//! policy loop and the UI all go through it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use protocol::discovery::{Announcement, Mode, Os, ShareStatus};
use protocol::models::{estimate_vram_bytes, CatalogEntry};
use serde::Serialize;
use tokio::sync::watch;

use crate::clamp::Limits;
use crate::engine::{EngineConfig, ModelSpec, Supervisor};
use crate::line::{JoinError, Line};
use crate::settings::HostSettings;
use crate::store::{Store, Usage};

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
/// How long a reply may keep going after a pause triggers (spec §6.3: "within a few seconds").
pub const PAUSE_GRACE: Duration = Duration::from_secs(3);
const TOAST_SECS: u64 = 6;

/// Static facts about this host, decided at startup.
#[derive(Debug, Clone)]
pub struct HostConfig {
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
    /// Shown to friends ("Dara's laptop").
    pub name: String,
    pub os: Os,
    pub app_version: String,
    /// The gatekeeper's port, as announced.
    pub port: u16,
    pub card: Option<Card>,
    pub engine: EngineConfig,
    pub catalog: Vec<CatalogEntry>,
    /// Show `dev_only` catalog entries (debug builds, or `GPU_SHARE_DEV=1`).
    pub dev_models: bool,
    /// Hosts models may be downloaded from.
    pub download_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub name: String,
    #[serde(rename = "vramGB")]
    pub vram_gb: f64,
}

/// What the policy loop decided. Spec §6.3.
#[derive(Debug, Clone, PartialEq)]
pub enum Gate {
    /// Sharing switched off: nothing announced, gatekeeper closed.
    Off,
    /// Paused. `owner` is the exact reason, `friend` the general one.
    Paused { owner: String, friend: String, countdown: bool },
    Open,
}

/// Where one queued request is, as its handler sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    Waiting(u32),
    Go,
    Stopped(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPhase {
    Queued,
    Loading,
    Streaming,
}

struct Slot {
    tx: watch::Sender<Turn>,
    phase: JobPhase,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinRefused {
    /// Paused or off; carries the friend-facing reason.
    Closed(String),
    AlreadyQueued,
}

struct Inner {
    settings: HostSettings,
    gate: Gate,
    line: Line<Slot>,
    last_request: Instant,
    toasts: Vec<(u64, String, String, Instant)>,
    toast_seq: u64,
}

pub struct HostCore {
    pub cfg: HostConfig,
    pub identity: netcore::Identity,
    pub store: Store,
    pub supervisor: Supervisor,
    pub pairing: crate::pairing::Pairing,
    /// The latest reading of the computer's state, for the owner's screen.
    signals: Mutex<crate::policy::Signals>,
    world: Mutex<Option<Arc<dyn crate::world::World>>>,
    /// Total RAM, read once from the world: the limit for hosts without a card.
    memory_gb: Mutex<Option<f64>>,
    /// The catalog in use: bundled or fetched, plus models added from a link, with shapes read
    /// from the files that are on disk.
    catalog: Mutex<Vec<CatalogEntry>>,
    downloads: Mutex<std::collections::HashMap<String, ViewDownload>>,
    /// Shown on the Share screen when the network is set up so friends can't connect.
    network_warning: Mutex<Option<String>>,
    /// First-run engine download (`P7-T3`): `Some` while it's in progress. Until it's done the
    /// host stays paused.
    engine_setup: Mutex<Option<ViewDownload>>,
    engine_ready: std::sync::atomic::AtomicBool,
    /// For model downloads: follows the system proxy, unlike `engine_http`.
    pub download_http: reqwest::Client,
    inner: Mutex<Inner>,
    settings_path: PathBuf,
    /// For the gatekeeper's engine requests: no proxy, no timeout (replies stream for a while).
    pub engine_http: reqwest::Client,
}

/// Dropping a ticket takes its request out of the line, whatever state it was in, and lets
/// the next one start. This is how Stop, Cancel and a vanished client all end up.
pub struct Ticket {
    pub id: u64,
    pub device: String,
    pub rx: watch::Receiver<Turn>,
    core: Arc<HostCore>,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.core.leave(self.id);
    }
}

impl Ticket {
    /// Records how far the running request has got, for the owner's Share screen.
    pub fn set_phase(&self, phase: JobPhase) {
        if let Some(slot) = self.core.inner.lock().unwrap().line.payload_mut(self.id) {
            slot.phase = phase;
        }
    }
}

impl HostCore {
    pub fn open(cfg: HostConfig) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(&cfg.data_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&cfg.models_dir).map_err(|e| e.to_string())?;
        let identity = netcore::Identity::load_or_create(&cfg.data_dir).map_err(|e| e.to_string())?;
        let store = Store::open(&cfg.data_dir.join("host.db")).map_err(|e| e.to_string())?;
        let settings_path = cfg.data_dir.join("settings.json");
        let mut settings = HostSettings::load(&settings_path);
        if !settings.resume_after_restart {
            settings.sharing = false; // spec §6.4: sharing starts off after a restart by default
        }
        let gate = if settings.sharing { Gate::Open } else { Gate::Off };
        let initial_catalog = cfg.catalog.clone();
        let core = Arc::new(Self {
            supervisor: Supervisor::new(cfg.engine.clone()),
            pairing: Default::default(),
            signals: Mutex::new(Default::default()),
            world: Mutex::new(None),
            memory_gb: Mutex::new(None),
            catalog: Mutex::new(vec![]),
            downloads: Mutex::new(Default::default()),
            network_warning: Mutex::new(None),
            engine_setup: Mutex::new(None),
            engine_ready: std::sync::atomic::AtomicBool::new(true),
            download_http: reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap(),
            cfg,
            identity,
            store,
            inner: Mutex::new(Inner {
                settings,
                gate,
                line: Line::new(),
                last_request: Instant::now(),
                toasts: vec![],
                toast_seq: 0,
            }),
            settings_path,
            engine_http: reqwest::Client::builder().no_proxy().build().unwrap(),
        });
        core.set_catalog(initial_catalog);
        Ok(core)
    }

    // ------------------------------------------------------------------ settings & models

    pub fn settings(&self) -> HostSettings {
        self.inner.lock().unwrap().settings.clone()
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut HostSettings)) {
        let mut inner = self.inner.lock().unwrap();
        let mut s = inner.settings.clone();
        f(&mut s);
        inner.settings = s.sanitized();
        if let Err(e) = inner.settings.save(&self.settings_path) {
            log::warn!("could not save settings: {e}");
        }
    }

    pub fn sharing(&self) -> bool {
        self.inner.lock().unwrap().settings.sharing
    }

    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.catalog.lock().unwrap().iter().filter(|e| self.cfg.dev_models || !e.dev_only).cloned().collect()
    }

    fn custom_path(&self) -> PathBuf {
        self.cfg.data_dir.join("custom_models.json")
    }

    /// Models added from a Hugging Face link, kept across restarts.
    fn custom_entries(&self) -> Vec<CatalogEntry> {
        std::fs::read(self.custom_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Replaces the catalog (bundled or fetched), keeping models added from links, and reads
    /// the real shape of every model already on disk.
    pub fn set_catalog(&self, list: Vec<CatalogEntry>) {
        let mut list = crate::library::validate(list, &self.cfg.download_hosts);
        for c in crate::library::validate(self.custom_entries(), &["huggingface.co".into()]) {
            if !list.iter().any(|e| e.id == c.id) {
                list.push(c);
            }
        }
        for e in &mut list {
            if let Ok(h) = crate::gguf::read_file(&self.cfg.models_dir.join(e.file_name())) {
                e.shape = Some(h.shape);
            }
        }
        *self.catalog.lock().unwrap() = list;
    }

    /// Adds a model from a link to the catalog (and remembers it).
    pub fn add_custom(&self, e: CatalogEntry) {
        let mut custom = self.custom_entries();
        custom.retain(|c| c.id != e.id);
        custom.push(e.clone());
        let _ = std::fs::write(self.custom_path(), serde_json::to_vec_pretty(&custom).unwrap());
        let mut cat = self.catalog.lock().unwrap();
        cat.retain(|c| c.id != e.id);
        cat.push(e);
    }

    /// After a download: the shape from the file itself replaces the catalog's guess (`P6-T6`).
    pub fn refresh_shape(&self, id: &str, path: &Path) {
        match crate::gguf::read_file(path) {
            Ok(h) => {
                if let Some(e) = self.catalog.lock().unwrap().iter_mut().find(|e| e.id == id) {
                    e.shape = Some(h.shape);
                }
            }
            Err(err) => log::warn!("{id}: {err}"),
        }
    }

    /// Marks the engine as being set up (`Some`) or ready (`None`).
    pub fn set_engine_setup(&self, d: Option<ViewDownload>) {
        self.engine_ready.store(d.is_none(), std::sync::atomic::Ordering::SeqCst);
        *self.engine_setup.lock().unwrap() = d;
    }

    /// The engine isn't on this computer yet: stay paused, without a progress banner until the
    /// download starts.
    pub fn set_engine_missing(&self) {
        self.engine_ready.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn engine_ready(&self) -> bool {
        self.engine_ready.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_network_warning(&self, w: Option<String>) {
        *self.network_warning.lock().unwrap() = w;
    }

    pub fn set_download(&self, id: &str, d: Option<ViewDownload>) {
        let mut all = self.downloads.lock().unwrap();
        match d {
            Some(d) => {
                all.insert(id.to_string(), d);
            }
            None => {
                all.remove(id);
            }
        }
    }

    pub fn downloading(&self, id: &str) -> bool {
        self.downloads.lock().unwrap().contains_key(id)
    }

    pub fn model_path(&self, e: &CatalogEntry) -> PathBuf {
        self.cfg.models_dir.join(e.file_name())
    }

    /// Catalog entries whose file is in the models folder.
    pub fn installed(&self) -> Vec<CatalogEntry> {
        self.catalog().into_iter().filter(|e| self.model_path(e).is_file()).collect()
    }

    /// Estimated VRAM in GB for a model at the current context size (spec §6.5).
    pub fn vram_gb(e: &CatalogEntry, ctx: u32) -> f64 {
        match e.shape {
            Some(shape) => estimate_vram_bytes(e.size_bytes, shape, ctx) as f64 / GB,
            None => e.size_bytes as f64 / GB + 0.5,
        }
    }

    /// Installed, switched on, and within the memory budget: what friends may pick. The owner's
    /// default model comes first, so it is what a friend's picker starts on (`P6-T7`).
    pub fn offered(&self) -> Vec<CatalogEntry> {
        let s = self.settings();
        let mut list: Vec<CatalogEntry> = self
            .installed()
            .into_iter()
            .filter(|e| !s.disabled.contains(&e.id) && Self::vram_gb(e, s.ctx) <= s.budget)
            .collect();
        if let Some(d) = &s.default_model {
            list.sort_by_key(|e| &e.id != d);
        }
        list
    }

    pub fn model_spec(&self, id: &str) -> Option<ModelSpec> {
        let s = self.settings();
        self.offered().into_iter().find(|e| e.id == id).map(|e| ModelSpec {
            path: self.model_path(&e),
            ctx: s.ctx,
            args: e.engine_args.clone(),
            size_bytes: e.size_bytes,
            id: e.id,
        })
    }

    pub fn limits(&self) -> Limits {
        Limits { max_reply: self.settings().max_reply }
    }

    // ------------------------------------------------------------------ signals

    pub fn attach_world(&self, world: Arc<dyn crate::world::World>) {
        *self.memory_gb.lock().unwrap() = world.memory_gb();
        *self.world.lock().unwrap() = Some(world);
    }

    pub fn world(&self) -> Option<Arc<dyn crate::world::World>> {
        self.world.lock().unwrap().clone()
    }

    pub fn set_signals(&self, s: crate::policy::Signals) {
        *self.signals.lock().unwrap() = s;
    }

    /// `P5-T5`: is there room on the card for this model right now? `None` when the card can't
    /// be asked (no NVML), in which case only the budget applies. The memory of a model that is
    /// loaded now counts as free, since it is stopped before the new one starts.
    pub fn vram_shortfall(&self, spec: &ModelSpec) -> Option<f64> {
        let free = self.world()?.free_vram_gb()?;
        let ctx = spec.ctx;
        let catalog = self.catalog();
        let entry = catalog.iter().find(|e| e.id == spec.id)?;
        let need = Self::vram_gb(entry, ctx);
        let released = self
            .supervisor
            .status()
            .loaded
            .and_then(|id| catalog.iter().find(|e| e.id == id))
            .map(|e| Self::vram_gb(e, ctx))
            .unwrap_or(0.0);
        let short = need - (free + released);
        (short > 0.0).then_some(short)
    }

    // ------------------------------------------------------------------ the gate

    pub fn gate(&self) -> Gate {
        self.inner.lock().unwrap().gate.clone()
    }

    /// Applies the policy loop's decision. Closing the gate turns away everyone waiting and
    /// tells the running request to stop (it gets [`PAUSE_GRACE`] to finish). Returns true
    /// when this call closed an open gate, so the caller can free the GPU after the grace.
    pub fn set_gate(&self, gate: Gate) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let was_open = inner.gate == Gate::Open;
        let closing = gate != Gate::Open;
        let friend = match &gate {
            Gate::Off => "Sharing was switched off".to_string(),
            Gate::Paused { friend, .. } => friend.clone(),
            Gate::Open => String::new(),
        };
        inner.gate = gate;
        if closing {
            for (_, slot) in inner.line.drain() {
                let _ = slot.tx.send(Turn::Stopped(friend.clone()));
            }
        } else {
            Self::pump(&mut inner);
        }
        was_open && closing
    }

    pub fn set_sharing(&self, on: bool) {
        self.update_settings(|s| s.sharing = on);
    }

    // ------------------------------------------------------------------ the queue

    pub fn join(self: &Arc<Self>, device: &str) -> Result<Ticket, JoinRefused> {
        let mut inner = self.inner.lock().unwrap();
        match &inner.gate {
            Gate::Off => return Err(JoinRefused::Closed("Sharing is switched off".into())),
            Gate::Paused { friend, .. } => return Err(JoinRefused::Closed(friend.clone())),
            Gate::Open => {}
        }
        let (tx, rx) = watch::channel(Turn::Waiting(0));
        let id = match inner.line.join(device, Slot { tx, phase: JobPhase::Queued }) {
            Ok(id) => id,
            Err(JoinError::AlreadyQueued) => return Err(JoinRefused::AlreadyQueued),
        };
        inner.last_request = Instant::now();
        Self::pump(&mut inner);
        Ok(Ticket { id, device: device.to_string(), rx, core: self.clone() })
    }

    fn leave(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.line.leave(id) {
            inner.last_request = Instant::now();
            Self::pump(&mut inner);
        }
    }

    /// Starts the next request if the card is free, and tells everyone waiting their place.
    fn pump(inner: &mut Inner) {
        if inner.gate == Gate::Open {
            if let Some(id) = inner.line.start_next() {
                if let Some(slot) = inner.line.payload(id) {
                    let _ = slot.tx.send(Turn::Go);
                }
            }
        }
        let positions: Vec<(u64, u32)> =
            inner.line.iter_waiting().filter_map(|(id, _)| inner.line.position(id).map(|p| (id, p))).collect();
        for (id, pos) in positions {
            if let Some(slot) = inner.line.payload(id) {
                slot.tx.send_if_modified(|t| {
                    let changed = *t != Turn::Waiting(pos);
                    *t = Turn::Waiting(pos);
                    changed
                });
            }
        }
    }

    /// The owner's Remove button (`P4-T5`): revokes the device and turns away its requests.
    pub fn unpair(&self, device: &str) {
        if let Err(e) = self.store.unpair(device) {
            log::error!("could not remove a device: {e}");
        }
        self.drop_device(device);
    }

    /// Turns away a removed device's requests.
    pub fn drop_device(&self, device: &str) {
        let mut inner = self.inner.lock().unwrap();
        for (_, slot) in inner.line.remove_device(device) {
            let _ = slot.tx.send(Turn::Stopped("The owner removed this computer".into()));
        }
        Self::pump(&mut inner);
    }

    pub fn idle_for(&self) -> Duration {
        let inner = self.inner.lock().unwrap();
        if inner.line.is_empty() {
            inner.last_request.elapsed()
        } else {
            Duration::ZERO
        }
    }

    pub fn busy(&self) -> bool {
        self.inner.lock().unwrap().line.running().is_some()
    }

    // ------------------------------------------------------------------ usage

    pub fn record_usage(&self, device: &str, usage: Usage) {
        if let Err(e) = self.store.add_usage(device, &today_utc(), usage) {
            log::warn!("could not record usage: {e}");
        }
    }

    // ------------------------------------------------------------------ toasts

    pub fn toast(&self, title: impl Into<String>, body: impl Into<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.toast_seq += 1;
        let id = inner.toast_seq;
        inner.toasts.push((id, title.into(), body.into(), Instant::now()));
    }

    // ------------------------------------------------------------------ what others see

    /// Status as announced to the LAN (`None` when off: an off host says nothing).
    pub fn share_status(&self) -> Option<(ShareStatus, Option<String>)> {
        let inner = self.inner.lock().unwrap();
        match &inner.gate {
            Gate::Off => None,
            Gate::Paused { friend, .. } => Some((ShareStatus::Paused, Some(friend.clone()))),
            Gate::Open if inner.line.running().is_some() => Some((ShareStatus::Busy, None)),
            Gate::Open => Some((ShareStatus::Available, None)),
        }
    }

    pub fn announcement(&self, announce: bool) -> Announcement {
        let status = self.share_status();
        Announcement {
            protocol: protocol::PROTOCOL_VERSION.into(),
            id: self.identity.fingerprint().into(),
            name: self.cfg.name.clone(),
            os: self.cfg.os,
            app_version: self.cfg.app_version.clone(),
            modes: vec![Mode::Share, Mode::Use],
            reason: status.as_ref().and_then(|s| s.1.clone()),
            status: status.map(|s| s.0),
            port: self.cfg.port,
            announce,
            gpu: self.cfg.card.as_ref().map(|c| c.name.clone()),
        }
    }

    /// The owner's Share screen (`HostView` in the UI).
    pub fn view(&self) -> HostView {
        let settings = self.settings();
        let engine = self.supervisor.status();
        let devices = self.store.devices().unwrap_or_default();
        let usage = self.store.usage_since(&days_ago_utc(7)).unwrap_or_default();
        let catalog = self.catalog();
        let installed: Vec<String> = self.installed().into_iter().map(|e| e.id).collect();
        let ctx = settings.ctx;
        let loaded_gb = |id: &Option<String>| {
            id.as_ref().and_then(|id| catalog.iter().find(|e| &e.id == id)).map(|e| Self::vram_gb(e, ctx))
        };
        let vram_used = loaded_gb(&engine.loaded)
            .or_else(|| loaded_gb(&engine.loading).map(|g| g * engine.load_pct as f64 / 100.0))
            .unwrap_or(0.0);

        // read these before taking `inner`: a guard created inside the struct literal below would
        // live until the end of it, and locking the same mutex twice there deadlocks
        let (temp, running_apps) = {
            let sig = self.signals.lock().unwrap();
            (sig.gpu_temp_c, sig.running_apps.clone())
        };
        let pending = self.pairing.pending().map(|p| ViewPending { id: p.device, name: p.name, code: p.code });
        let network_warning = self.network_warning.lock().unwrap().clone();
        let idle_known = self.signals.lock().unwrap().idle_secs.is_some();
        let memory_gb = *self.memory_gb.lock().unwrap();
        let engine_setup = self.engine_setup.lock().unwrap().clone();

        let mut inner = self.inner.lock().unwrap();
        inner.toasts.retain(|t| t.3.elapsed() < Duration::from_secs(TOAST_SECS));
        let (state, reason, client_reason, countdown) = match &inner.gate {
            Gate::Off => ("off", String::new(), String::new(), false),
            Gate::Paused { owner, friend, countdown } => ("paused", owner.clone(), friend.clone(), *countdown),
            Gate::Open if inner.line.running().is_some() => ("busy", String::new(), String::new(), false),
            Gate::Open => ("available", String::new(), String::new(), false),
        };
        let job = inner.line.running().map(|(_, device, slot)| ViewJob {
            who: device.to_string(),
            phase: if slot.phase == JobPhase::Streaming { JobPhase::Streaming } else { JobPhase::Loading },
        });
        HostView {
            name: self.cfg.name.clone(),
            card: self.cfg.card.clone(),
            sharing: settings.sharing,
            state: state.into(),
            reason,
            client_reason,
            countdown,
            loaded: engine.loaded.clone(),
            loading: engine.loading.clone(),
            load_pct: engine.load_pct,
            vram_used_gb: vram_used,
            temp,
            job,
            queue: inner.line.waiting_devices(),
            paired: devices.iter().map(|d| ViewDevice { id: d.id.clone(), name: d.name.clone() }).collect(),
            usage: usage
                .into_iter()
                .map(|(id, u)| {
                    (
                        id,
                        ViewUsage {
                            reqs: u.requests,
                            tok_in: u.tokens_in,
                            tok_out: u.tokens_out,
                            gpu_sec: u.gpu_ms as f64 / 1000.0,
                        },
                    )
                })
                .collect(),
            pending,
            catalog: catalog.iter().map(ViewModel::from_entry).collect(),
            installed,
            downloads: self.downloads.lock().unwrap().clone(),
            toasts: inner
                .toasts
                .iter()
                .map(|t| ViewToast { id: format!("t{}", t.0), title: t.1.clone(), body: t.2.clone() })
                .collect(),
            running_apps,
            network_warning,
            engine_setup,
            idle_known,
            memory_gb,
            settings,
        }
    }
}

// ---------------------------------------------------------------------- view DTOs

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub name: String,
    pub card: Option<Card>,
    pub sharing: bool,
    pub settings: HostSettings,
    pub state: String,
    pub reason: String,
    pub client_reason: String,
    pub countdown: bool,
    pub loaded: Option<String>,
    pub loading: Option<String>,
    pub load_pct: u8,
    #[serde(rename = "vramUsedGB")]
    pub vram_used_gb: f64,
    pub temp: Option<f64>,
    pub job: Option<ViewJob>,
    pub queue: Vec<String>,
    pub paired: Vec<ViewDevice>,
    pub usage: std::collections::HashMap<String, ViewUsage>,
    pub pending: Option<ViewPending>,
    pub catalog: Vec<ViewModel>,
    pub installed: Vec<String>,
    pub downloads: std::collections::HashMap<String, ViewDownload>,
    pub toasts: Vec<ViewToast>,
    pub running_apps: Vec<String>,
    pub network_warning: Option<String>,
    pub engine_setup: Option<ViewDownload>,
    /// Whether this computer can tell when the owner is using it (false on Linux desktops other
    /// than GNOME, until the first reading).
    pub idle_known: bool,
    /// Total RAM, for hosts that answer on the processor.
    #[serde(rename = "memoryGB")]
    pub memory_gb: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewJob {
    pub who: String,
    pub phase: JobPhase,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewUsage {
    pub reqs: u64,
    pub tok_in: u64,
    pub tok_out: u64,
    pub gpu_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewPending {
    pub id: String,
    pub name: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewDownload {
    pub pct: f64,
    pub stage: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewToast {
    pub id: String,
    pub title: String,
    pub body: String,
}

/// A catalog entry as the Models screen shows it (`CatalogEntry` in the UI).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewModel {
    pub id: String,
    pub name: String,
    pub quant: String,
    #[serde(rename = "fileGB")]
    pub file_gb: f64,
    #[serde(rename = "kvGB")]
    pub kv_gb: f64,
    pub tps: u32,
    pub desc: String,
    pub license: String,
}

impl ViewModel {
    pub fn from_entry(e: &CatalogEntry) -> Self {
        let kv4k = e
            .shape
            .map(|s| (estimate_vram_bytes(0, s, 4096) - estimate_vram_bytes(0, s, 0)) as f64 / GB)
            .unwrap_or(0.0);
        Self {
            id: e.id.clone(),
            name: e.name.clone(),
            quant: e.quantization.clone(),
            file_gb: e.size_bytes as f64 / GB,
            kv_gb: kv4k,
            tps: e.tps_estimate.unwrap_or(0),
            desc: e.description.clone(),
            license: e.license.clone(),
        }
    }
}

// ---------------------------------------------------------------------- dates

fn days_since_epoch(t: SystemTime) -> i64 {
    (t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400) as i64
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn ymd(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn today_utc() -> String {
    ymd(days_since_epoch(SystemTime::now()))
}

fn days_ago_utc(n: i64) -> String {
    ymd(days_since_epoch(SystemTime::now()) - n)
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

/// Reads the bundled catalog.
pub fn bundled_catalog() -> Vec<CatalogEntry> {
    serde_json::from_str(include_str!("../catalog.json")).expect("bundled catalog.json is valid")
}

pub fn default_models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(20_739), "2026-10-13");
        assert_eq!(ymd(11_016), "2000-02-29");
    }

    #[test]
    fn the_bundled_catalog_parses_and_the_8b_fits_the_default_budget() {
        let cat = bundled_catalog();
        let q8 = cat.iter().find(|e| e.id == "qwen3-8b-q4km").unwrap();
        let gb = HostCore::vram_gb(q8, 4096);
        assert!(gb < 6.0 && gb > 5.0, "{gb}");
        assert!(cat.iter().all(|e| e.sha256.len() == 64 && e.file_url.starts_with("https://huggingface.co/")));
    }
}
