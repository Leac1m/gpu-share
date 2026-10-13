//! Announcing the host on the LAN (`P3-T2`, `P3-T3`, `P3-T8`, spec §6.6).
//!
//! While sharing is on, the host multicasts its announcement on every status change and every
//! [`HEARTBEAT_SECS`], and answers a client's hello (an announcement with `announce: true`)
//! straight back to it with `announce: false`. While sharing is off it sends nothing at all and
//! ignores hellos: an off host is invisible.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use netcore::lan::{lan_interfaces, multicast_socket, send_multicast};
use protocol::discovery::{Announcement, ShareStatus, HEARTBEAT_SECS, MULTICAST_PORT};

use crate::core::HostCore;

type Status = Option<(ShareStatus, Option<String>)>;

/// When to announce. Pure, so the "off means silent" rule is testable without a network.
#[derive(Debug, Default)]
pub struct Schedule {
    last: Option<(Instant, Status)>,
}

impl Schedule {
    /// True when an announcement should go out now.
    pub fn due(&mut self, now: Instant, status: &Status) -> bool {
        if status.is_none() {
            self.last = None; // the next time sharing is on, announce at once
            return false;
        }
        let send = match &self.last {
            None => true,
            Some((at, prev)) => prev != status || now.duration_since(*at) >= Duration::from_secs(HEARTBEAT_SECS),
        };
        if send {
            self.last = Some((now, status.clone()));
        }
        send
    }
}

/// Runs forever. Spawn it alongside the host runtime.
pub async fn run(core: Arc<HostCore>) {
    let mut ifaces = lan_interfaces();
    let sock = match multicast_socket(&ifaces) {
        Ok(s) => s,
        Err(e) => {
            log::error!("discovery socket: {e}; the host can still be added by IP address");
            return;
        }
    };
    let mut schedule = Schedule::default();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut refreshed = Instant::now();
    let mut buf = vec![0u8; 4096];
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if refreshed.elapsed() > Duration::from_secs(60) {
                    ifaces = lan_interfaces(); // laptops change networks
                    refreshed = Instant::now();
                }
                let status = core.share_status();
                if schedule.due(Instant::now(), &status) {
                    let a = serde_json::to_vec(&core.announcement(true)).unwrap();
                    send_multicast(&sock, &ifaces, &a).await;
                }
            }
            r = sock.recv_from(&mut buf) => {
                let Ok((n, from)) = r else { continue };
                let Ok(hello) = serde_json::from_slice::<Announcement>(&buf[..n]) else { continue };
                if hello.id == core.identity.fingerprint() || !hello.announce || core.share_status().is_none() {
                    continue;
                }
                // reply straight to whoever said hello, so it doesn't wait for the heartbeat
                let reply = serde_json::to_vec(&core.announcement(false)).unwrap();
                let _ = sock.send_to(&reply, SocketAddr::new(from.ip(), MULTICAST_PORT)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AVAILABLE: Status = Some((ShareStatus::Available, None));

    #[test]
    fn an_off_host_never_announces() {
        let mut s = Schedule::default();
        let t = Instant::now();
        for i in 0..100 {
            assert!(!s.due(t + Duration::from_secs(i), &None));
        }
    }

    #[test]
    fn announces_at_once_then_every_heartbeat() {
        let mut s = Schedule::default();
        let t = Instant::now();
        assert!(s.due(t, &AVAILABLE));
        assert!(!s.due(t + Duration::from_secs(5), &AVAILABLE));
        assert!(s.due(t + Duration::from_secs(HEARTBEAT_SECS), &AVAILABLE));
    }

    #[test]
    fn every_status_change_is_announced_straight_away() {
        let mut s = Schedule::default();
        let t = Instant::now();
        s.due(t, &AVAILABLE);
        assert!(s.due(t + Duration::from_secs(1), &Some((ShareStatus::Busy, None))));
        let paused = Some((ShareStatus::Paused, Some("They're using their computer".into())));
        assert!(s.due(t + Duration::from_secs(2), &paused));
        assert!(!s.due(t + Duration::from_secs(3), &paused));
    }

    #[test]
    fn switching_back_on_announces_immediately() {
        let mut s = Schedule::default();
        let t = Instant::now();
        s.due(t, &AVAILABLE);
        s.due(t + Duration::from_secs(1), &None);
        assert!(s.due(t + Duration::from_secs(2), &AVAILABLE));
    }
}
