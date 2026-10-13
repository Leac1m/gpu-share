//! Chat through the real gatekeeper and the fake engine: streaming, the line, Stop, pauses,
//! model switches and usage counts (`P2-T6`, `P2-T8`, `P2-T9`, `P2-T10`).

mod common;
use common::*;

use std::time::Duration;

use client::api::{ChatEvent, ChatStream};
use futures_util::StreamExt;
use protocol::api::{ErrorKind, QueueEvent};

async fn next_queue_event(s: &mut ChatStream) -> QueueEvent {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), s.next()).await.expect("event in time") {
            Some(Ok(ChatEvent::Queue(q))) => return q,
            Some(Ok(_)) => continue,
            other => panic!("stream ended early: {other:?}"),
        }
    }
}

async fn reply_text(mut s: ChatStream) -> (String, Vec<QueueEvent>) {
    let mut text = String::new();
    let mut events = vec![];
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            ChatEvent::Delta(t) => text.push_str(&t),
            ChatEvent::Queue(q) => events.push(q),
            ChatEvent::Usage { .. } => {}
        }
    }
    (text, events)
}

#[tokio::test]
async fn a_reply_streams_through_the_gatekeeper() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let (text, events) = reply_text(me.api.chat(&chat_req("model-a", "hello there")).await.unwrap()).await;
    assert_eq!(text.trim(), "Echo: hello there");
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Loading { .. })), "first question loads the model");
    assert!(events.contains(&QueueEvent::Started));
}

#[tokio::test]
async fn waiting_clients_are_told_their_place_and_take_turns() {
    let host = start_host(&[("FAKE_TOKEN_MS", "60")]).await;
    let tunde = host.client(true);
    let ada = host.client(true);
    let me = host.client(true);

    let mut t = tunde.api.chat(&chat_req("model-a", "one two three four five six seven eight")).await.unwrap();
    assert!(matches!(next_queue_event(&mut t).await, QueueEvent::Loading { .. } | QueueEvent::Started));
    let mut a = ada.api.chat(&chat_req("model-a", "second")).await.unwrap();
    assert_eq!(next_queue_event(&mut a).await, QueueEvent::Queued { position: 2 });
    let mut m = me.api.chat(&chat_req("model-a", "third")).await.unwrap();
    assert_eq!(next_queue_event(&mut m).await, QueueEvent::Queued { position: 3 });

    // the host's view shows who is waiting, never what they asked
    let view = host.core.view();
    assert_eq!(view.queue.len(), 2);
    assert!(!serde_json::to_string(&view).unwrap().contains("third"));

    let (t_text, _) = reply_text(t).await;
    assert!(t_text.contains("eight"));
    assert_eq!(next_queue_event(&mut m).await, QueueEvent::Queued { position: 2 }, "moves up when the first finishes");
    let (a_text, _) = reply_text(a).await;
    assert!(a_text.contains("second"));
    let (m_text, _) = reply_text(m).await;
    assert!(m_text.contains("third"));
}

#[tokio::test]
async fn one_waiting_request_per_device() {
    let host = start_host(&[("FAKE_TOKEN_MS", "50")]).await;
    let me = host.client(true);
    let _first = me.api.chat(&chat_req("model-a", "a b c d e f")).await.unwrap();
    let second = me.api.chat(&chat_req("model-a", "again")).await;
    assert_eq!(second.err().unwrap().api_kind(), Some(ErrorKind::AlreadyQueued));
}

#[tokio::test]
async fn stop_frees_the_card_for_the_next_friend() {
    let host = start_host(&[("FAKE_TOKEN_MS", "200")]).await;
    let me = host.client(true);
    let other = host.client(true);
    let mut mine = me.api.chat(&chat_req("model-a", "a long answer with many many words in it")).await.unwrap();
    while next_queue_event(&mut mine).await != QueueEvent::Started {}
    let mut theirs = other.api.chat(&chat_req("model-a", "next")).await.unwrap();
    assert_eq!(next_queue_event(&mut theirs).await, QueueEvent::Queued { position: 2 });
    drop(mine); // Stop
    let (text, _) = tokio::time::timeout(Duration::from_secs(10), reply_text(theirs)).await.unwrap();
    assert!(text.contains("next"));
    // and the stopped device can ask again straight away
    assert!(me.api.chat(&chat_req("model-a", "again")).await.is_ok());
}

#[tokio::test]
async fn cancelling_while_in_line_leaves_the_line() {
    let host = start_host(&[("FAKE_TOKEN_MS", "100")]).await;
    let a = host.client(true);
    let b = host.client(true);
    let c = host.client(true);
    let _sa = a.api.chat(&chat_req("model-a", "one two three four five six")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut sb = b.api.chat(&chat_req("model-a", "b")).await.unwrap();
    assert_eq!(next_queue_event(&mut sb).await, QueueEvent::Queued { position: 2 });
    let mut sc = c.api.chat(&chat_req("model-a", "c")).await.unwrap();
    assert_eq!(next_queue_event(&mut sc).await, QueueEvent::Queued { position: 3 });
    drop(sb); // Cancel
    assert_eq!(next_queue_event(&mut sc).await, QueueEvent::Queued { position: 2 });
}

#[tokio::test]
async fn a_pause_turns_away_everyone_waiting_and_cuts_the_running_reply() {
    let host = start_host(&[("FAKE_TOKEN_MS", "100")]).await;
    let a = host.client(true);
    let b = host.client(true);
    let words: String = (0..200).map(|i| format!("w{i} ")).collect();
    let mut sa = a.api.chat(&chat_req("model-a", &words)).await.unwrap();
    while next_queue_event(&mut sa).await != QueueEvent::Started {}
    let mut sb = b.api.chat(&chat_req("model-a", "b")).await.unwrap();
    assert_eq!(next_queue_event(&mut sb).await, QueueEvent::Queued { position: 2 });

    host.core.set_gate(host::core::Gate::Paused {
        owner: "Blender is open".into(),
        friend: "They're using their computer".into(),
        countdown: false,
    });
    let reason = QueueEvent::Stopped { reason: "They're using their computer".into() };
    assert_eq!(next_queue_event(&mut sb).await, reason, "waiting friends get the general reason");
    let started = std::time::Instant::now();
    let (text, events) = reply_text(sa).await;
    assert!(events.contains(&reason), "the running reply ends with the reason, not a raw error");
    assert!(started.elapsed() < Duration::from_secs(6), "cut within a few seconds");
    assert!(!text.contains("w199"), "the reply was cut, not finished");

    let refused = b.api.chat(&chat_req("model-a", "now?")).await.err().unwrap();
    assert_eq!(refused.api_kind(), Some(ErrorKind::Paused));
    assert_eq!(refused.to_string(), "They're using their computer", "never the owner's program names");
}

#[tokio::test]
async fn a_different_model_swaps_the_engine_process() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    reply_text(me.api.chat(&chat_req("model-a", "x")).await.unwrap()).await;
    let pid_a = host.core.supervisor.pid().await.unwrap();
    let (_, events) = reply_text(me.api.chat(&chat_req("model-b", "y")).await.unwrap()).await;
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Loading { model, .. } if model == "model-b")));
    assert_ne!(host.core.supervisor.pid().await.unwrap(), pid_a);
    assert_eq!(host.core.supervisor.status().loaded.as_deref(), Some("model-b"));
}

#[tokio::test]
async fn usage_is_counted_per_device() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    for q in ["one", "two"] {
        reply_text(me.api.chat(&chat_req("model-a", q)).await.unwrap()).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let usage = host.core.store.usage_since("2000-01-01").unwrap();
    let (_, u) = usage.iter().find(|(id, _)| id == me.id.fingerprint()).unwrap();
    assert_eq!(u.requests, 2);
    assert!(u.tokens_out > 0 && u.tokens_in > 0 && u.gpu_ms > 0);
}

#[tokio::test]
async fn unknown_models_are_refused_before_queueing() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let e = me.api.chat(&chat_req("../model-a", "x")).await.err().unwrap();
    assert_eq!(e.api_kind(), Some(ErrorKind::UnknownModel));
}

#[tokio::test]
async fn the_engines_file_path_never_reaches_the_client() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let r = raw(&host, &me.id)
        .post(format!("https://{}/v1/chat/completions", host.addr))
        .bearer_auth(format!("token-{}", &me.id.fingerprint()[..8]))
        .json(&serde_json::json!({"model":"model-a","messages":[{"role":"user","content":"x"}],"stream":true}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(r.contains("\"model\":\"model-a\""));
    assert!(!r.contains(".gguf"), "{r}");
}

#[tokio::test]
async fn non_streaming_clients_get_one_completion() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let v: serde_json::Value = raw(&host, &me.id)
        .post(format!("https://{}/v1/chat/completions", host.addr))
        .bearer_auth(format!("token-{}", &me.id.fingerprint()[..8]))
        .json(&serde_json::json!({"model":"model-a","messages":[{"role":"user","content":"plain"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["choices"][0]["message"]["content"].as_str().unwrap().trim(), "Echo: plain");
}
