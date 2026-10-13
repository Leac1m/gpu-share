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
