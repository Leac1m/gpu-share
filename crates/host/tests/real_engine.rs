//! End to end with the real llama-server and a real model (`P2` "done when"). Needs the dev
//! engine from `scripts/dev-engine.sh`, so it is ignored by default:
//!
//!     scripts/dev-engine.sh && cargo test -p host --test real_engine -- --ignored

mod common;
use common::*;

use std::path::PathBuf;

use futures_util::StreamExt;

fn dev_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.dev")
}

#[tokio::test]
#[ignore = "needs .dev/llama-server and the dev model (scripts/dev-engine.sh)"]
async fn a_real_model_answers_through_the_gatekeeper() {
    let catalog = host::core::bundled_catalog();
    let smol = catalog.iter().find(|e| e.id == "smollm2-135m-q8").unwrap().clone();
    let model_file = dev_dir().join("models/SmolLM2-135M-Instruct-Q8_0.gguf");
    let host = start_host_with(&[], dev_dir().join("llama-server"), vec![smol.clone()], vec![(smol.id.clone(), model_file)]).await;
    let me = host.client(true);

    let models = me.api.models().await.unwrap();
    assert_eq!(models.data[0].id, "smollm2-135m-q8");

    let mut req = chat_req("smollm2-135m-q8", "What is the capital of France? Answer in one word.");
    req.max_tokens = Some(20);
    let mut s = me.api.chat(&req).await.unwrap();
    let mut text = String::new();
    let mut usage = None;
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            client::api::ChatEvent::Delta(t) => text.push_str(&t),
            client::api::ChatEvent::Usage { completion_tokens, .. } => usage = Some(completion_tokens),
            _ => {}
        }
    }
    println!("reply: {text:?}, completion tokens: {usage:?}");
    assert!(!text.trim().is_empty());
    assert!(usage.unwrap() <= 20, "max_tokens was honoured");
    host.core.supervisor.stop().await;
}
