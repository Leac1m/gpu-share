//! Roadmap suite 5 (spec §6.8, §7): run chats carrying a magic string through the gatekeeper
//! and the engine, then search every file the host wrote for it.

mod common;
use common::*;

use futures_util::StreamExt;

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[tokio::test]
async fn prompts_and_replies_never_touch_the_hosts_disk() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    const MAGIC: &str = "ZebraQuokka7731";
    for i in 0..3 {
        let mut s = me.api.chat(&chat_req("model-a", &format!("{MAGIC} question {i}"))).await.unwrap();
        let mut reply = String::new();
        while let Some(ev) = s.next().await {
            if let client::api::ChatEvent::Delta(t) = ev.unwrap() {
                reply.push_str(&t);
            }
        }
        assert!(reply.contains(MAGIC), "the echo engine sends the magic string back in the reply");
    }
    // the stopped-mid-way path too
    let mut s = me.api.chat(&chat_req("model-a", &format!("{MAGIC} {}", "word ".repeat(50)))).await.unwrap();
    let _ = s.next().await;
    drop(s);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // the view the owner sees doesn't carry it either
    assert!(!serde_json::to_string(&host.core.view()).unwrap().contains(MAGIC));

    let files = files_under(host.dir.path());
    assert!(files.iter().any(|f| f.ends_with("host.db")), "the usage database was written");
    for f in files {
        if f.starts_with(host.dir.path().join("models")) {
            continue; // the (fake) model files
        }
        let bytes = std::fs::read(&f).unwrap();
        assert!(
            !bytes.windows(MAGIC.len()).any(|w| w == MAGIC.as_bytes()),
            "{} contains prompt or reply text",
            f.display()
        );
    }
}
