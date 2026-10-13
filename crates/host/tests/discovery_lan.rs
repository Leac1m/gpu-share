//! Multicast discovery on this machine's real interfaces (`P3-T2`, `P3-T8`). Needs a LAN-ish
//! interface; skipped (passes trivially) where there is none.

mod common;
use common::*;

use std::time::{Duration, Instant};

use client::discovery::Discovery;
use client::history::History;
use client::runtime::ClientRuntime;
use host::core::Gate;
use protocol::discovery::{Announcement, Mode, Os};

async fn seen_within(rt: &ClientRuntime, id: &str, d: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < d {
        if rt.hosts().iter().any(|h| h.info.id == id) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn an_off_host_is_invisible_and_an_on_host_appears() {
    if netcore::lan::lan_interfaces().is_empty() {
        // e.g. a cloud VM on a non-private address: use its first real interface anyway
        match netcore::lan::all_interfaces().into_iter().find(|i| !i.ip.is_loopback()) {
            Some(i) => std::env::set_var("GPU_SHARE_IFACE", &i.name),
            None => {
                eprintln!("no network interface here; skipping");
                return;
            }
        }
    }
    let host = start_host(&[]).await;
    host.core.set_gate(Gate::Off);
    tokio::spawn(host::announce::run(host.core.clone()));

    let id = netcore::Identity::generate().unwrap();
    let me = Announcement {
        protocol: protocol::PROTOCOL_VERSION.into(),
        id: id.fingerprint().into(),
        name: "Client".into(),
        os: Os::Linux,
        app_version: "0".into(),
        modes: vec![Mode::Use],
        status: None,
        port: protocol::GATEKEEPER_PORT,
        announce: true,
        reason: None,
        gpu: None,
    };
    let rt = ClientRuntime::new(id, History::in_memory().unwrap(), "Client".into());
    let disco = Discovery::new(rt.clone(), me);
    tokio::spawn(disco.clone().run());

    let host_id = host.core.identity.fingerprint().to_string();
    assert!(!seen_within(&rt, &host_id, Duration::from_secs(3)).await, "sharing off: nothing announced");

    host.core.set_gate(Gate::Open);
    assert!(seen_within(&rt, &host_id, Duration::from_secs(4)).await, "sharing on: announced within a tick");
    let seen = rt.hosts().into_iter().find(|h| h.info.id == host_id).unwrap();
    assert_eq!(seen.info.status, Some(protocol::discovery::ShareStatus::Available));
    assert!(disco.observations().heard_multicast);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_subnet_scan_finds_a_host_multicast_never_announced() {
    // All of 127.0.0.0/8 is loopback on Linux, so a /24 scan from 127.0.0.9 finds the test
    // gatekeeper on 127.0.0.1 and gets refusals from the other 252 addresses.
    let host = start_host(&[]).await;
    let id = netcore::Identity::generate().unwrap();
    let me = Announcement {
        protocol: protocol::PROTOCOL_VERSION.into(),
        id: id.fingerprint().into(),
        name: "Client".into(),
        os: Os::Linux,
        app_version: "0".into(),
        modes: vec![Mode::Use],
        status: None,
        port: host.addr.port(),
        announce: true,
        reason: None,
        gpu: None,
    };
    let rt = ClientRuntime::new(id, History::in_memory().unwrap(), "Client".into());
    let disco = Discovery::with_port(rt.clone(), me, host.addr.port());
    let lo = netcore::lan::Iface { name: "lo".into(), ip: "127.0.0.9".parse().unwrap(), netmask: "255.255.255.0".parse().unwrap() };
    disco.scan(&[lo]).await;
    assert!(rt.hosts().iter().any(|h| h.info.id == host.core.identity.fingerprint()));
    let o = disco.observations();
    assert!(o.scanned && o.scan_responders > 200, "{o:?}");
}
