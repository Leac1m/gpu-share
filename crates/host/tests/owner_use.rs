//! The owner using his own GPU from the same app (`P9-T1`): a listener on 127.0.0.1 only, for
//! this install's own certificate, whose questions skip the pause rules and go ahead of friends.

mod common;
use common::*;

use std::time::Duration;

use client::api::{ChatEvent, ChatStream, HostClient};
use futures_util::StreamExt;
use host::core::Gate;
use host::gatekeeper::Gatekeeper;
use netcore::tls::ServerPin;
use protocol::api::{ErrorKind, QueueEvent};

struct Owner {
    _gk: Gatekeeper,
    api: HostClient,
    addr: std::net::SocketAddr,
    token: String,
}

async fn owner(host: &TestHost) -> Owner {
    let token = "owner-secret".to_string();
    let gk = Gatekeeper::start_owner(host.core.clone(), token.clone()).await.unwrap();
    let addr = gk.addr;
    let api = HostClient::new(&host.core.identity, addr, ServerPin::Fingerprint(host.core.identity.fingerprint().into()), Some(token.clone())).unwrap();
    Owner { _gk: gk, api, addr, token }
}

async fn next_queue_event(s: &mut ChatStream) -> QueueEvent {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), s.next()).await.expect("event in time") {
            Some(Ok(ChatEvent::Queue(q))) => return q,
            Some(Ok(_)) => continue,
            other => panic!("stream ended early: {other:?}"),
        }
    }
}

async fn reply_text(mut s: ChatStream) -> String {
    let mut text = String::new();
    while let Some(ev) = s.next().await {
        if let ChatEvent::Delta(t) = ev.unwrap() {
            text.push_str(&t);
        }
    }
    text
}

#[tokio::test]
async fn the_owner_listener_is_on_loopback_only() {
    let host = start_host(&[]).await;
    let o = owner(&host).await;
    assert!(o.addr.ip().is_loopback());
}

#[tokio::test]
async fn the_owner_can_chat_with_sharing_off_or_paused() {
    let host = start_host(&[]).await;
    let o = owner(&host).await;
    host.core.set_gate(Gate::Off);
    assert_eq!(reply_text(o.api.chat(&chat_req("model-a", "off")).await.unwrap()).await.trim(), "Echo: off");
    host.core.set_gate(Gate::Paused { owner: "You're using your computer".into(), friend: "They're busy".into(), countdown: false });
    assert_eq!(reply_text(o.api.chat(&chat_req("model-a", "paused")).await.unwrap()).await.trim(), "Echo: paused");
    // friends are still turned away
    let friend = host.client(true);
    let err = friend.api.chat(&chat_req("model-a", "hi")).await.err().expect("refused");
    assert_eq!(err.api_kind(), Some(ErrorKind::Paused));
}

#[tokio::test]
async fn the_owner_goes_ahead_of_friends_waiting_but_not_the_one_being_answered() {
    let host = start_host(&[("FAKE_TOKEN_MS", "60")]).await;
    let o = owner(&host).await;
    let tunde = host.client(true);
    let ada = host.client(true);
    let mut t = tunde.api.chat(&chat_req("model-a", "one two three four five six seven eight nine ten")).await.unwrap();
    assert!(matches!(next_queue_event(&mut t).await, QueueEvent::Loading { .. } | QueueEvent::Started));
    let mut a = ada.api.chat(&chat_req("model-a", "second")).await.unwrap();
    assert_eq!(next_queue_event(&mut a).await, QueueEvent::Queued { position: 2 });
    let mut me = o.api.chat(&chat_req("model-a", "mine")).await.unwrap();
    assert_eq!(next_queue_event(&mut me).await, QueueEvent::Queued { position: 2 }, "right after the running reply");
    assert_eq!(next_queue_event(&mut a).await, QueueEvent::Queued { position: 3 }, "Ada moves back one");
    assert_eq!(reply_text(me).await.trim(), "Echo: mine");
}

#[tokio::test]
async fn a_pause_turns_friends_away_but_lets_the_owner_finish() {
    let host = start_host(&[("FAKE_TOKEN_MS", "80")]).await;
    let o = owner(&host).await;
    let friend = host.client(true);
    let mut me = o.api.chat(&chat_req("model-a", "a b c d e f g h i j k l m n o p")).await.unwrap();
    assert!(matches!(next_queue_event(&mut me).await, QueueEvent::Loading { .. } | QueueEvent::Started));
    let mut f = friend.api.chat(&chat_req("model-a", "waiting")).await.unwrap();
    assert_eq!(next_queue_event(&mut f).await, QueueEvent::Queued { position: 2 });
    host.core.set_gate(Gate::Paused { owner: "Blender is open".into(), friend: "They're busy".into(), countdown: false });
    assert!(matches!(next_queue_event(&mut f).await, QueueEvent::Stopped { .. }), "the friend is turned away");
    let text = reply_text(me).await;
    assert!(text.trim().ends_with('p'), "the owner's reply is not cut: {text:?}");
}

#[tokio::test]
async fn the_owner_sees_models_kept_from_friends() {
    let host = start_host(&[]).await;
    let o = owner(&host).await;
    host.core.update_settings(|s| s.disabled = vec!["model-b".into()]);
    let friend = host.client(true);
    let ids = |l: protocol::api::ModelList| l.data.into_iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(ids(friend.api.models().await.unwrap()), vec!["model-a"]);
    assert_eq!(ids(o.api.models().await.unwrap()), vec!["model-a", "model-b"]);
    assert_eq!(reply_text(o.api.chat(&chat_req("model-b", "mine")).await.unwrap()).await.trim(), "Echo: mine");
    assert!(friend.api.chat(&chat_req("model-b", "x")).await.is_err(), "still not for friends");
}

#[tokio::test]
async fn the_owners_questions_are_not_counted_as_a_friends_usage() {
    let host = start_host(&[]).await;
    let o = owner(&host).await;
    reply_text(o.api.chat(&chat_req("model-a", "count me")).await.unwrap()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(host.core.view().usage.is_empty(), "{:?}", host.core.view().usage);
}

#[tokio::test]
async fn only_this_installs_certificate_with_the_token_gets_in() {
    let host = start_host(&[]).await;
    let o = owner(&host).await;
    let pin = || ServerPin::Fingerprint(host.core.identity.fingerprint().into());
    // a paired friend, even with its own valid token, is not the owner
    let friend = host.client(true);
    let token = format!("token-{}", &friend.id.fingerprint()[..8]); // what `client(true)` paired it with
    let as_friend = HostClient::new(&friend.id, o.addr, pin(), Some(token)).unwrap();
    assert_eq!(as_friend.models().await.err().and_then(|e| e.api_kind()), Some(ErrorKind::Unauthorized));
    // the owner's certificate with the wrong token
    let wrong = HostClient::new(&host.core.identity, o.addr, pin(), Some("guess".into())).unwrap();
    assert_eq!(wrong.models().await.err().and_then(|e| e.api_kind()), Some(ErrorKind::Unauthorized));
    // a stranger with the right token but its own certificate
    let stranger = netcore::Identity::generate().unwrap();
    let s = HostClient::new(&stranger, o.addr, pin(), Some(o.token.clone())).unwrap();
    assert_eq!(s.models().await.err().and_then(|e| e.api_kind()), Some(ErrorKind::Unauthorized));
    // pairing and discovery are not offered here at all
    let http = {
        let tls = netcore::tls::client_config(&host.core.identity, pin()).unwrap();
        reqwest::Client::builder().use_preconfigured_tls(tls).no_proxy().build().unwrap()
    };
    for path in ["/api/pair", "/api/register"] {
        let r = http.post(format!("https://{}{path}", o.addr)).body("{}").send().await.unwrap();
        assert_eq!(r.status(), 404, "{path}");
    }
    assert!(o.api.info().await.is_ok());
}

/// The policy loop frees the card when a pause starts, but not from under the owner's own
/// answer; once he's done, it does.
#[tokio::test]
async fn a_pause_frees_the_card_only_after_the_owners_answer() {
    let host = start_host(&[("FAKE_TOKEN_MS", "80")]).await;
    let o = owner(&host).await;
    let world_file = host.dir.path().join("world.json");
    std::fs::write(&world_file, "{}").unwrap();
    let rt = host::runtime::HostRuntime::new(
        host.core.clone(),
        "127.0.0.1:0".parse().unwrap(),
        std::sync::Arc::new(host::world::FileWorld { path: world_file.clone() }),
    );
    host.core.set_sharing(true);
    rt.tick().await;
    assert_eq!(host.core.gate(), Gate::Open);

    let words: Vec<String> = (0..60).map(|i| format!("w{i}")).collect();
    let mut me = o.api.chat(&chat_req("model-a", &words.join(" "))).await.unwrap();
    while next_queue_event(&mut me).await != QueueEvent::Started {}
    std::fs::write(&world_file, r#"{"running_apps": ["Blender"]}"#).unwrap();
    rt.tick().await;
    assert!(matches!(host.core.gate(), Gate::Paused { .. }));
    tokio::time::sleep(Duration::from_secs(4)).await; // past the friends' grace
    rt.tick().await;
    assert!(host.core.supervisor.status().loaded.is_some(), "the owner's model stays loaded");
    let text = reply_text(me).await;
    assert!(text.trim().ends_with("w59"), "not cut: {text:?}");

    tokio::time::sleep(Duration::from_millis(300)).await;
    rt.tick().await;
    assert_eq!(host.core.supervisor.status().loaded, None, "freed once the owner is done");
}
