//! The host's background loop (`P5-T1`): every second it reads the computer's state, runs the
//! policy, and turns the decision into a running (or closed) gatekeeper. It also notices engine
//! crashes, frees the card when idle or over budget, and keeps the PC awake while serving.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::awake::KeepAwake;
use crate::core::{Gate, HostCore, PAUSE_GRACE};
use crate::gatekeeper::Gatekeeper;
use crate::policy::{self, PolicyState};
use crate::world::World;

pub const TICK: Duration = Duration::from_secs(1);

pub struct HostRuntime {
    pub core: Arc<HostCore>,
    bind: SocketAddr,
    gatekeeper: tokio::sync::Mutex<Option<Gatekeeper>>,
    world: Arc<dyn World>,
    policy: Mutex<PolicyState>,
    awake: KeepAwake,
    ticks: std::sync::atomic::AtomicU64,
}

impl HostRuntime {
    pub fn new(core: Arc<HostCore>, bind: SocketAddr, world: Arc<dyn World>) -> Arc<Self> {
        core.attach_world(world.clone());
        Arc::new(Self {
            core,
            bind,
            gatekeeper: tokio::sync::Mutex::new(None),
            world,
            policy: Mutex::new(PolicyState::default()),
            awake: KeepAwake::new(),
            ticks: Default::default(),
        })
    }

    /// Runs the loop forever. Spawn it.
    pub async fn run(self: Arc<Self>) {
        loop {
            self.tick().await;
            tokio::time::sleep(TICK).await;
        }
    }

    pub async fn tick(&self) {
        // 1. read the computer (process lists and counters can take a few ms: off the async threads)
        if let Some(pid) = self.core.supervisor.pid().await {
            self.world.set_own_pids(vec![pid]);
        }
        let world = self.world.clone();
        let signals = tokio::task::spawn_blocking(move || world.sample()).await.unwrap_or_default();
        let mut gate = policy::decide(&self.core.settings(), &signals, &mut self.policy.lock().unwrap());
        self.core.set_signals(signals);
        if gate == Gate::Open && !self.core.engine_ready() {
            gate = Gate::Paused {
                owner: "Getting the AI engine ready (a one-time download)".into(),
                friend: "Getting ready. Back in a few minutes".into(),
                countdown: true, // no "paused" toast for this
            };
        }

        // 2. apply the decision
        if let Gate::Paused { owner, countdown: false, .. } = &gate {
            if !matches!(self.core.gate(), Gate::Paused { .. }) {
                let freed = if self.core.cfg.card.is_some() { "Your graphics card is free again" } else { "The model was unloaded" };
                self.core.toast("Sharing paused", format!("{owner}. {freed}."));
            }
        }
        let off = gate == Gate::Off;
        let closed = self.core.set_gate(gate.clone());

        // the port is open exactly while sharing is on (a paused host still answers "paused")
        let mut gk = self.gatekeeper.lock().await;
        if off {
            if let Some(g) = gk.take() {
                g.stop().await;
            }
        } else if gk.is_none() {
            match Gatekeeper::start(self.core.clone(), self.bind).await {
                Ok(g) => *gk = Some(g),
                Err(e) => {
                    log::error!("gatekeeper could not start on {}: {e}", self.bind);
                    self.core.toast("Sharing couldn't start", format!("Port {} is in use by another program.", self.bind.port()));
                    self.core.set_sharing(false);
                }
            }
        }
        drop(gk);

        // 3. a pause or switch-off frees the card once the running reply has had its grace
        if closed {
            let core = self.core.clone();
            tokio::spawn(async move {
                tokio::time::sleep(PAUSE_GRACE + Duration::from_millis(500)).await;
                if core.gate() != Gate::Open {
                    core.supervisor.stop().await;
                }
            });
        }

        // 4. engine health, idle, budget
        if self.core.supervisor.check().await {
            self.core.toast("The model stopped", "It will start again with the next question.");
        }
        if let Some(err) = self.core.supervisor.take_error() {
            log::warn!("engine: {err}");
        }
        let settings = self.core.settings();
        let loaded = self.core.supervisor.status().loaded;
        if loaded.is_some() && self.core.idle_for() > Duration::from_secs(settings.idle_free_min as u64 * 60) {
            self.core.supervisor.stop().await;
            self.core.toast("Graphics card freed", "No one has used it for a while, so the model was unloaded.");
        } else if let Some(id) = &loaded {
            let over = self
                .core
                .catalog()
                .iter()
                .find(|e| &e.id == id)
                .is_some_and(|e| HostCore::vram_gb(e, settings.ctx) > settings.budget);
            if over && !self.core.busy() {
                self.core.supervisor.stop().await;
                self.core.toast("Model unloaded", "It needs more memory than your limit allows.");
            }
        }

        // 5. every half minute: can friends reach this computer? Windows "Public" networks
        // (P7-T2) and Ubuntu's ufw (P8-T6)
        if self.ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(30) {
            let public = tokio::task::spawn_blocking(netcore::lan::network_is_public).await.unwrap_or(None);
            let warning = netcore::lan::public_network_warning(public)
                .or_else(|| netcore::lan::host_firewall_warning(netcore::lan::ufw_enabled(), self.core.cfg.port));
            self.core.set_network_warning(warning);
        }

        // 6. keep the PC awake only while actually serving (P5-T9)
        let serving = self.core.gate() == Gate::Open && (self.core.busy() || self.core.supervisor.status().loaded.is_some());
        self.awake.set(serving);
    }

    pub fn keeping_awake(&self) -> bool {
        self.awake.is_on()
    }

    pub async fn shutdown(&self) {
        if let Some(g) = self.gatekeeper.lock().await.take() {
            g.stop().await;
        }
        self.core.supervisor.stop().await;
        self.awake.set(false);
    }
}
