//! Pairing end to end (`P4-T2`–`P4-T5`): the popup, the code on both screens, Allow / Not now,
//! and a man in the middle showing different codes.

mod common;
use common::*;

use std::sync::Arc;
use std::time::Duration;

use client::history::History;
use client::runtime::ClientRuntime;
use netcore::tls::ServerPin;
use netcore::Identity;
use protocol::api::ErrorKind;
use protocol::pairing::{pairing_code, PairRequest};
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;

async fn wait_pending(host: &TestHost) -> host::pairing::Pending {
    for _ in 0..100 {
        if let Some(p) = host.core.pairing.pending() {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no pairing popup appeared");
}

fn runtime() -> Arc<ClientRuntime> {
    ClientRuntime::new(Identity::generate().unwrap(), History::in_memory().unwrap(), "Tunde's PC".into())
}

#[tokio::test]
async fn allow_pairs_the_device_and_both_screens_show_the_same_code() {
    let host = start_host(&[]).await;
    let rt = runtime();
    rt.add_by_ip(&host.addr.to_string()).await.unwrap();
    let host_id = host.core.identity.fingerprint().to_string();

    rt.ask_connect(&host_id);
    let popup = wait_pending(&host).await;
    let (_, client_code) = rt.pairing().unwrap();
    assert_eq!(popup.code, client_code, "the owner and the friend see the same six digits");
    assert_eq!(popup.name, "Tunde's PC");
    assert_eq!(popup.device, rt.identity.fingerprint());
    assert!(host.core.view().pending.is_some(), "the popup is in the owner's view");

    // nothing is granted before Allow
    assert!(host.core.store.devices().unwrap().is_empty());

    host.core.pairing.decide(true);
    for _ in 0..100 {
        if rt.is_paired(&host_id) && rt.pairing().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(rt.is_paired(&host_id));
    assert_eq!(host.core.store.devices().unwrap()[0].name, "Tunde's PC");
    let v = rt.view();
    assert_eq!(v.chat_host.as_deref(), Some(host_id.as_str()), "pairing opens the chat");
    assert_eq!(v.hosts[0].models.len(), 2, "and fetches the models");

    // the owner removes the device (P4-T5): it can no longer chat
    host.core.unpair(rt.identity.fingerprint());
    rt.refresh().await;
    assert!(!rt.is_paired(&host_id));
}

#[tokio::test]
async fn not_now_grants_nothing_and_says_so() {
    let host = start_host(&[]).await;
    let rt = runtime();
    rt.add_by_ip(&host.addr.to_string()).await.unwrap();
    rt.ask_connect(host.core.identity.fingerprint());
    wait_pending(&host).await;
    host.core.pairing.decide(false);
    for _ in 0..100 {
        if !rt.view().notice.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(rt.view().notice, "The owner said not now.");
    assert!(rt.view().pairing.is_none());
    assert!(host.core.store.devices().unwrap().is_empty());
}

#[tokio::test]
async fn one_request_at_a_time_and_a_vanished_client_clears_the_popup() {
    let host = start_host(&[]).await;
    let a = host.client(false);
    let b = host.client(false);
    let req = |n: &str| PairRequest { protocol: "1.0".into(), name: n.into(), nonce: client::api::new_nonce() };
    let pending_a = {
        let api = a.api.clone();
        let r = req("A");
        tokio::spawn(async move { api.pair(&r).await })
    };
    wait_pending(&host).await;
    let e = b.api.pair(&req("B")).await.unwrap_err();
    assert_eq!(e.api_kind(), Some(ErrorKind::Busy));

    pending_a.abort(); // A gives up (closes the app, loses the network)
    for _ in 0..100 {
        if host.core.pairing.pending().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(host.core.pairing.pending().is_none(), "the popup goes away with the request");
}

/// A TLS-terminating proxy: shows its own certificate to the client and its own to the host,
/// and passes the plaintext through. This is the attack pairing codes exist to catch.
async fn mitm(host: &TestHost) -> (std::net::SocketAddr, String) {
    let me = Identity::generate().unwrap();
    let fp = me.fingerprint().to_string();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(netcore::tls::server_config(&me).unwrap()));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(
        netcore::tls::client_config(&me, ServerPin::Capture(Default::default())).unwrap(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = host.addr;
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (acceptor, connector) = (acceptor.clone(), connector.clone());
            tokio::spawn(async move {
                let Ok(mut down) = acceptor.accept(tcp).await else { return };
                let up = tokio::net::TcpStream::connect(upstream).await.unwrap();
                let Ok(mut up) = connector.connect(netcore::tls::server_name(), up).await else { return };
                let _ = copy_bidirectional(&mut down, &mut up).await;
            });
        }
    });
    (addr, fp)
}

#[tokio::test]
async fn a_proxy_claiming_to_be_the_host_is_not_added() {
    let host = start_host(&[]).await;
    let (proxy, _) = mitm(&host).await;
    let rt = runtime();
    let err = rt.add_by_ip(&proxy.to_string()).await.unwrap_err();
    assert!(err.contains("didn't check out"), "{err}");
    assert!(rt.hosts().is_empty());
}

#[tokio::test]
async fn a_man_in_the_middle_shows_different_codes() {
    // A smarter attacker gets the friend to pin *its* certificate (say, by announcing its own
    // id). Pairing through it, each side computes the code from what it actually sees.
    let host = start_host(&[]).await;
    let (proxy, proxy_fp) = mitm(&host).await;
    let friend = Identity::generate().unwrap();
    let nonce = client::api::new_nonce();
    let api = client::api::HostClient::new(&friend, proxy, ServerPin::Fingerprint(proxy_fp.clone()), None).unwrap();
    let req = PairRequest { protocol: "1.0".into(), name: "Victim".into(), nonce: nonce.clone() };
    let pairing = tokio::spawn(async move { api.pair(&req).await });

    let popup = wait_pending(&host).await;
    let friends_screen = pairing_code(&proxy_fp, friend.fingerprint(), &nonce);
    assert_eq!(popup.device, proxy_fp, "the host sees the proxy's certificate, not the friend's");
    assert_ne!(popup.code, friends_screen, "so the owner's code doesn't match the friend's");

    host.core.pairing.decide(false);
    let _ = pairing.await;
}
