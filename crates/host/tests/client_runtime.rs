//! The client runtime (what the Use-mode screens read) against a real gatekeeper (`P2-T8`).

mod common;
use common::*;

use std::sync::Arc;
use std::time::Duration;

use client::history::{History, KnownHost};
use client::runtime::{ClientRuntime, RemoteModel, Turn};

async fn paired_runtime(host: &TestHost) -> Arc<ClientRuntime> {
    let id = netcore::Identity::generate().unwrap();
    host.core.store.pair(id.fingerprint(), "My PC", "tok", 1).unwrap();
    let history = History::in_memory().unwrap();
    history
        .save_host(&KnownHost {
            id: host.core.identity.fingerprint().into(),
            name: "Test host".into(),
            token: "tok".into(),
            address: host.addr.to_string(),
        })
        .unwrap();
    let rt = ClientRuntime::new(id, history, "My PC".into());
    let info = rt.api(host.core.identity.fingerprint(), host.addr).unwrap().info().await.unwrap();
    rt.upsert_host(info, host.addr);
    let host_id = host.core.identity.fingerprint().to_string();
    rt.set_models(&host_id, vec![RemoteModel { id: "model-a".into(), name: "Model A".into() }]);
    rt.open_chat(&host_id);
    rt.set_model("model-a");
    rt
}

async fn wait_for(rt: &ClientRuntime, f: impl Fn(&client::runtime::ClientView) -> bool) -> client::runtime::ClientView {
    for _ in 0..200 {
        let v = rt.view();
        if f(&v) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out; last view: {:?}", rt.view());
}

#[tokio::test]
async fn a_question_streams_into_the_view_and_lands_in_history() {
    let host = start_host(&[("FAKE_TOKEN_MS", "30")]).await;
    let rt = paired_runtime(&host).await;
    let v = rt.view();
    assert_eq!(v.hosts.len(), 1);
    assert!(v.hosts[0].paired);

    rt.send("hello from the client");
    let v = wait_for(&rt, |v| v.turn == Some(Turn::Streaming) && v.msgs.last().is_some_and(|m| m.streaming)).await;
    assert_eq!(v.msgs[0].role, "user");
    let v = wait_for(&rt, |v| v.turn.is_none()).await;
    let last = v.msgs.last().unwrap();
    assert_eq!(last.role, "assistant");
    assert_eq!(last.text.trim(), "Echo: hello from the client");
    assert!(!last.streaming);

    // the next question carries the conversation so far
    rt.send("second");
    let v = wait_for(&rt, |v| v.turn.is_none() && v.msgs.len() == 4).await;
    assert!(v.msgs[3].text.contains("second"));
}

#[tokio::test]
async fn stop_keeps_what_arrived_and_says_so() {
    let host = start_host(&[("FAKE_TOKEN_MS", "150")]).await;
    let rt = paired_runtime(&host).await;
    rt.send(&"many words ".repeat(30));
    wait_for(&rt, |v| v.msgs.last().is_some_and(|m| m.streaming && m.text.len() > 5)).await;
    rt.stop();
    let v = wait_for(&rt, |v| v.turn.is_none()).await;
    assert_eq!(v.msgs.last().unwrap().text, "You stopped the answer.");
    assert_eq!(v.msgs[v.msgs.len() - 2].role, "assistant", "the partial reply is kept");
    // the host is free again for this device
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!host.core.busy());
}

#[tokio::test]
async fn a_paused_host_gives_a_note_not_an_error() {
    let host = start_host(&[]).await;
    let rt = paired_runtime(&host).await;
    host.core.set_gate(host::core::Gate::Paused { owner: "Steam is open".into(), friend: "They're using their computer".into(), countdown: false });
    rt.send("anyone there?");
    let v = wait_for(&rt, |v| v.turn.is_none() && v.msgs.len() == 2).await;
    assert_eq!(
        v.msgs[1].text,
        "Your question wasn't answered. They're using their computer. Send it again when the host is back."
    );
}

#[tokio::test]
async fn add_by_ip_finds_the_host_and_checks_its_identity() {
    let host = start_host(&[]).await;
    let rt = ClientRuntime::new(netcore::Identity::generate().unwrap(), History::in_memory().unwrap(), "Me".into());
    let name = rt.add_by_ip(&host.addr.to_string()).await.unwrap();
    assert_eq!(name, "Test host");
    assert_eq!(rt.hosts()[0].info.id, host.core.identity.fingerprint());
    assert!(!rt.view().hosts[0].paired);

    let err = rt.add_by_ip("127.0.0.1:1").await.unwrap_err();
    assert!(err.contains("Nothing answered"), "{err}");
    let err = rt.add_by_ip("not an address").await.unwrap_err();
    assert!(err.contains("doesn't look like an address"), "{err}");
}

#[tokio::test]
async fn a_host_that_switches_off_disappears_after_two_polls() {
    let mut host = start_host(&[]).await;
    let rt = ClientRuntime::new(netcore::Identity::generate().unwrap(), History::in_memory().unwrap(), "Me".into());
    rt.add_by_ip(&host.addr.to_string()).await.unwrap();
    rt.refresh().await;
    assert_eq!(rt.hosts().len(), 1);
    host.gk.take().unwrap().stop().await; // sharing off: the port closes
    rt.refresh().await;
    assert_eq!(rt.hosts().len(), 1, "one missed poll is forgiven");
    rt.refresh().await;
    assert!(rt.hosts().is_empty(), "two and it's gone");
}

#[tokio::test]
async fn ask_again_after_a_pause_sends_the_same_question() {
    let host = start_host(&[]).await;
    let rt = paired_runtime(&host).await;
    host.core.set_gate(host::core::Gate::Paused { owner: "x".into(), friend: "They're using their computer".into(), countdown: false });
    rt.send("the question");
    wait_for(&rt, |v| v.turn.is_none() && v.msgs.len() == 2).await;
    host.core.set_gate(host::core::Gate::Open);
    rt.retry();
    let v = wait_for(&rt, |v| v.turn.is_none() && v.msgs.last().is_some_and(|m| m.role == "assistant")).await;
    assert!(v.msgs.last().unwrap().text.contains("the question"));
}

/// `P9-T1`: the owner's own computer is on his list as "This computer", whether sharing is on
/// or off, and answers him through its loopback listener.
#[tokio::test]
async fn the_owners_own_computer_is_always_on_his_list_and_answers_him() {
    let host = start_host(&[]).await;
    host.core.set_gate(host::core::Gate::Off);
    let gk = host::gatekeeper::Gatekeeper::start_owner(host.core.clone(), "t0ken".into()).await.unwrap();
    let rt = ClientRuntime::new(host.core.identity.clone(), History::in_memory().unwrap(), "Owner".into());
    rt.set_own_host(gk.addr, "t0ken".into(), host.core.announcement(true));
    rt.refresh().await;
    rt.expire(Duration::ZERO);
    let v = rt.view();
    assert_eq!(v.hosts.len(), 1);
    let me = &v.hosts[0];
    assert!(me.own && me.paired);
    assert_eq!(me.name, "This computer");
    assert_eq!(me.status, "available", "sharing is off, but not for the owner");
    assert_eq!(me.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["model-a", "model-b"]);

    rt.forget(&me.id);
    assert_eq!(rt.view().hosts.len(), 1, "can't be forgotten");
    rt.open_chat(&me.id);
    rt.set_model("model-a");
    rt.send("just me");
    let v = wait_for(&rt, |v| v.turn.is_none() && v.msgs.last().is_some_and(|m| m.role == "assistant")).await;
    assert_eq!(v.msgs.last().unwrap().text.trim(), "Echo: just me");
}
