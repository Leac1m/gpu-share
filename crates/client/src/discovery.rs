//! Finding hosts on the LAN (`P3-T2`–`P3-T4`, `P3-T7`, spec §6.6).
//!
//! * Listens for announcements on the multicast group and says hello once on startup (and every
//!   minute), which makes hosts answer at once instead of at their next heartbeat.
//! * When a host is first heard, replies over HTTPS with `POST /api/register`. If that fails the
//!   host can be heard but not reached, which usually means a firewall.
//! * If nothing has turned up a few seconds in, scans the local /24 for the gatekeeper port.
//! * Keeps [`Observations`] so an empty list comes with a plain-language reason.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use netcore::lan::{self, Iface, Observations};
use protocol::discovery::Announcement;
use tokio::sync::Notify;

use crate::runtime::ClientRuntime;

const FIRST_SCAN_AFTER: Duration = Duration::from_secs(8);
const HELLO_EVERY: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);
const PROBE_CONCURRENCY: usize = 64;

pub struct Discovery {
    rt: Arc<ClientRuntime>,
    me: Mutex<Announcement>,
    obs: Mutex<Observations>,
    scan_now: Notify,
    /// The gatekeeper port the subnet scan probes.
    port: u16,
}

impl Discovery {
    /// `me` is this computer's own announcement (its id must be the identity's fingerprint).
    pub fn new(rt: Arc<ClientRuntime>, me: Announcement) -> Arc<Self> {
        Self::with_port(rt, me, protocol::GATEKEEPER_PORT)
    }

    pub fn with_port(rt: Arc<ClientRuntime>, me: Announcement, port: u16) -> Arc<Self> {
        Arc::new(Self { rt, me: Mutex::new(me), obs: Mutex::new(Observations::default()), scan_now: Notify::new(), port })
    }

    /// Ask for a subnet scan ("Look again").
    pub fn rescan(&self) {
        self.scan_now.notify_one();
    }

    pub fn observations(&self) -> Observations {
        self.obs.lock().unwrap().clone()
    }

    /// Runs forever. Spawn it.
    pub async fn run(self: Arc<Self>) {
        let ifaces = lan::lan_interfaces();
        {
            let mut o = self.obs.lock().unwrap();
            o.interfaces = ifaces.len();
            o.ufw_enabled = lan::ufw_enabled();
        }
        let scanner = {
            let me = self.clone();
            let ifaces = ifaces.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(FIRST_SCAN_AFTER) => {}
                    _ = me.scan_now.notified() => {}
                }
                loop {
                    if me.rt.hosts().is_empty() || !me.observations().scanned {
                        me.scan(&ifaces).await;
                    }
                    me.scan_now.notified().await;
                }
            })
        };
        let hinter = {
            let me = self.clone();
            tokio::spawn(async move {
                let mut public = None;
                let mut n = 0u64;
                loop {
                    if n.is_multiple_of(15) {
                        public = tokio::task::spawn_blocking(lan::network_is_public).await.unwrap_or(None);
                    }
                    n += 1;
                    let hint = if me.rt.hosts().is_empty() {
                        lan::public_network_warning(public).or_else(|| lan::diagnose(&me.observations())).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    me.rt.set_network_hint(hint);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            })
        };
        self.listen(&ifaces).await;
        scanner.abort();
        hinter.abort();
    }

    async fn listen(self: &Arc<Self>, ifaces: &[Iface]) {
        let sock = match lan::multicast_socket(ifaces) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("discovery socket: {e}; relying on the subnet scan and Add by IP");
                std::future::pending::<()>().await;
                return;
            }
        };
        let mut hello = tokio::time::interval(HELLO_EVERY);
        let mut buf = vec![0u8; 4096];
        loop {
            tokio::select! {
                _ = hello.tick() => {
                    let me = serde_json::to_vec(&*self.me.lock().unwrap()).unwrap();
                    lan::send_multicast(&sock, ifaces, &me).await;
                }
                r = sock.recv_from(&mut buf) => {
                    let Ok((n, from)) = r else { continue };
                    let Ok(a) = serde_json::from_slice::<Announcement>(&buf[..n]) else { continue };
                    self.obs.lock().unwrap().heard_multicast = true;
                    if a.id == self.me.lock().unwrap().id {
                        continue; // our own hello, looped back: multicast works here
                    }
                    self.heard(a, SocketAddr::new(from.ip(), protocol::GATEKEEPER_PORT));
                }
            }
        }
    }

    fn heard(self: &Arc<Self>, a: Announcement, from: SocketAddr) {
        let addr = SocketAddr::new(from.ip(), a.port);
        let new = !self.rt.hosts().iter().any(|h| h.info.id == a.id);
        let reply = a.announce && new && a.status.is_some();
        let (id, name) = (a.id.clone(), a.name.clone());
        self.rt.upsert_host(a, addr);
        if reply {
            let me = self.clone();
            tokio::spawn(async move {
                let Ok(api) = me.rt.api(&id, addr) else { return };
                let hello = me.me.lock().unwrap().clone();
                if api.register(&hello).await.is_err() {
                    me.rt.set_network_hint(format!(
                        "{name} can be seen but not reached. Its firewall may be blocking GPU Share: on Windows, the \
                         network should be set to Private."
                    ));
                }
            });
        }
    }

    /// `GET /api/info` across the local /24 (spec §6.6 fallback).
    pub async fn scan(self: &Arc<Self>, ifaces: &[Iface]) {
        let targets: Vec<IpAddr> = ifaces.iter().flat_map(lan::subnet_24).map(IpAddr::V4).collect();
        let responders = futures_util::stream::iter(targets)
            .map(|ip| {
                let me = self.clone();
                async move { me.probe(ip).await }
            })
            .buffer_unordered(PROBE_CONCURRENCY)
            .filter(|answered| futures_util::future::ready(*answered))
            .count()
            .await;
        let mut o = self.obs.lock().unwrap();
        o.scanned = true;
        o.scan_responders = responders;
    }

    /// True if anything at `ip` answered, even with a refusal (a device is there). The router
    /// usually answers too, so `.1` and `.254` don't count towards "devices can see each other".
    async fn probe(&self, ip: IpAddr) -> bool {
        let addr = SocketAddr::new(ip, self.port);
        let gateway_like = matches!(ip, IpAddr::V4(v4) if matches!(v4.octets()[3], 1 | 254));
        match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(_)) => {
                if let Err(e) = self.rt.add_by_ip(&addr.to_string()).await {
                    log::info!("{addr} answered but isn't a sharing host: {e}");
                }
                !gateway_like
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => !gateway_like,
            _ => false,
        }
    }
}
