//! The host's background loop: turns the sharing switch and the policy loop's decision into a
//! running (or closed) gatekeeper, notices engine crashes, and frees the card when idle.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::core::{Gate, HostCore, PAUSE_GRACE};
use crate::gatekeeper::Gatekeeper;

/// Decides the gate from the switch and whatever signals are available. Phase 5 replaces the
/// body with the real policy loop.
pub trait Policy: Send + Sync + 'static {
    fn decide(&self, core: &HostCore) -> Gate;
}

/// Open whenever sharing is on.
pub struct SwitchOnly;

impl Policy for SwitchOnly {
    fn decide(&self, core: &HostCore) -> Gate {
        if core.sharing() {
            Gate::Open
        } else {
            Gate::Off
        }
    }
}

pub const TICK: Duration = Duration::from_secs(1);

pub struct HostRuntime {
    pub core: Arc<HostCore>,
    bind: SocketAddr,
    gatekeeper: Mutex<Option<Gatekeeper>>,
    policy: Box<dyn Policy>,
}

impl HostRuntime {
    pub fn new(core: Arc<HostCore>, bind: SocketAddr, policy: Box<dyn Policy>) -> Arc<Self> {
        Arc::new(Self { core, bind, gatekeeper: Mutex::new(None), policy })
    }

    /// Runs the loop forever. Spawn it.
    pub async fn run(self: Arc<Self>) {
        loop {
            self.tick().await;
            tokio::time::sleep(TICK).await;
        }
    }

    pub async fn tick(&self) {
        let gate = self.policy.decide(&self.core);
        let off = gate == Gate::Off;
        let paused_now = matches!(gate, Gate::Paused { .. }) && !matches!(self.core.gate(), Gate::Paused { .. });
        if let Gate::Paused { owner, .. } = &gate {
            if paused_now {
                self.core.toast("Sharing paused", format!("{owner}. Your graphics card is free again."));
            }
        }
        let closed = self.core.set_gate(gate.clone());

        // the port is open exactly while sharing is on (paused hosts still say they're paused)
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
                    self.core.toast(
                        "Sharing couldn't start",
                        format!("Port {} is in use by another program.", self.bind.port()),
                    );
                    self.core.set_sharing(false);
                }
            }
        }
        drop(gk);

        // a pause or switch-off frees the card once the running reply has had its grace
        if closed {
            let core = self.core.clone();
            tokio::spawn(async move {
                tokio::time::sleep(PAUSE_GRACE + Duration::from_millis(500)).await;
                if core.gate() != Gate::Open {
                    core.supervisor.stop().await;
                }
            });
        }

        if self.core.supervisor.check().await {
            self.core.toast("The model stopped", "It will start again with the next question.");
        }
        if let Some(err) = self.core.supervisor.take_error() {
            log::warn!("engine: {err}");
        }

        // free the card after a quiet spell (spec §6.4)
        let idle_limit = Duration::from_secs(self.core.settings().idle_free_min as u64 * 60);
        if self.core.supervisor.status().loaded.is_some() && self.core.idle_for() > idle_limit {
            self.core.supervisor.stop().await;
            self.core.toast("Graphics card freed", "No one has used it for a while, so the model was unloaded.");
        }
    }

    pub async fn shutdown(&self) {
        if let Some(g) = self.gatekeeper.lock().await.take() {
            g.stop().await;
        }
        self.core.supervisor.stop().await;
    }
}
