//! Roadmap suite 10: downloads resume after a dropped connection and are only accepted with the
//! right SHA-256. Against a local server, no internet needed.

mod common;
use common::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use host::library::{download, DownloadError};
use protocol::models::CatalogEntry;
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct Server {
    body: Arc<Vec<u8>>,
    /// Cut the first response off after this many bytes.
    drop_after: Arc<Mutex<Option<usize>>>,
    honour_range: Arc<AtomicBool>,
    ranges: Arc<Mutex<Vec<String>>>,
    hits: Arc<AtomicUsize>,
}

async fn serve_file(State(s): State<Server>, headers: HeaderMap) -> Response {
    s.hits.fetch_add(1, Ordering::SeqCst);
    let range = headers.get("range").and_then(|v| v.to_str().ok()).map(String::from);
    let mut start = 0;
    let mut status = StatusCode::OK;
    if let Some(r) = &range {
        s.ranges.lock().unwrap().push(r.clone());
        if s.honour_range.load(Ordering::SeqCst) {
            start = r.trim_start_matches("bytes=").trim_end_matches('-').parse().unwrap();
            status = StatusCode::PARTIAL_CONTENT;
        }
    }
    let mut data = s.body[start..].to_vec();
    let cut = s.drop_after.lock().unwrap().take();
    if let Some(n) = cut {
        // send n bytes, then fail the stream: the client sees a broken transfer
        data.truncate(n);
        let stream = futures_util::stream::iter(vec![
            Ok::<_, std::io::Error>(bytes::Bytes::from(data)),
            Err(std::io::Error::other("connection dropped")),
        ]);
        return Response::builder().status(status).body(axum::body::Body::from_stream(stream)).unwrap();
    }
    (status, data).into_response()
}

async fn server(body: Vec<u8>) -> (String, Server) {
    let s = Server {
        body: Arc::new(body),
        drop_after: Default::default(),
        honour_range: Arc::new(AtomicBool::new(true)),
        ranges: Default::default(),
        hits: Default::default(),
    };
    let app = axum::Router::new().route("/model.gguf", get(serve_file)).with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{addr}/model.gguf"), s)
}

/// A real GGUF header (so the shape can be read after download) padded to a few MB.
fn model_bytes() -> Vec<u8> {
    let s = |v: &str| {
        let mut b = (v.len() as u64).to_le_bytes().to_vec();
        b.extend(v.as_bytes());
        b
    };
    let kv = |k: &str, ty: u32, v: Vec<u8>| {
        let mut b = s(k);
        b.extend(ty.to_le_bytes());
        b.extend(v);
        b
    };
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend(5u64.to_le_bytes());
    b.extend(kv("general.architecture", 8, s("llama")));
    b.extend(kv("llama.block_count", 4, 24u32.to_le_bytes().to_vec()));
    b.extend(kv("llama.embedding_length", 4, 2048u32.to_le_bytes().to_vec()));
    b.extend(kv("llama.attention.head_count", 4, 16u32.to_le_bytes().to_vec()));
    b.extend(kv("llama.attention.head_count_kv", 4, 4u32.to_le_bytes().to_vec()));
    b.extend((0..3_000_000u32).map(|i| (i % 251) as u8));
    b
}

fn entry_for(url: &str, body: &[u8]) -> CatalogEntry {
    let mut e = entry("dl-model");
    e.file_url = url.into();
    e.size_bytes = body.len() as u64;
    e.sha256 = hex::encode(Sha256::digest(body));
    e.shape = None;
    e
}

fn plenty(_: &std::path::Path) -> Option<u64> {
    Some(u64::MAX / 2)
}

#[tokio::test]
async fn a_download_is_verified_and_lands_in_the_models_folder() {
    let body = model_bytes();
    let (url, _) = server(body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let e = entry_for(&url, &body);
    let seen = Mutex::new(vec![]);
    let path = download(&reqwest::Client::new(), &e, dir.path(), plenty, |d, t| seen.lock().unwrap().push((d, t))).await.unwrap();
    assert_eq!(path, dir.path().join("dl-model.gguf"));
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert!(!host::library::part_path(dir.path(), &e).exists());
    assert_eq!(seen.lock().unwrap().last().unwrap().0, body.len() as u64);
}

#[tokio::test]
async fn a_dropped_connection_resumes_where_it_stopped() {
    let body = model_bytes();
    let (url, srv) = server(body.clone()).await;
    *srv.drop_after.lock().unwrap() = Some(1_000_000);
    let dir = tempfile::tempdir().unwrap();
    let e = entry_for(&url, &body);
    let http = reqwest::Client::new();
    let first = download(&http, &e, dir.path(), plenty, |_, _| {}).await;
    assert!(matches!(first, Err(DownloadError::Network(_))), "{first:?}");
    let part = host::library::part_path(dir.path(), &e);
    assert_eq!(std::fs::metadata(&part).unwrap().len(), 1_000_000, "what arrived is kept");

    let path = download(&http, &e, dir.path(), plenty, |_, _| {}).await.unwrap();
    assert_eq!(srv.ranges.lock().unwrap().as_slice(), ["bytes=1000000-"], "asked only for the rest");
    assert_eq!(std::fs::read(path).unwrap(), body, "and the whole file still checks out");
}

#[tokio::test]
async fn a_server_that_ignores_the_range_starts_over_cleanly() {
    let body = model_bytes();
    let (url, srv) = server(body.clone()).await;
    *srv.drop_after.lock().unwrap() = Some(500_000);
    srv.honour_range.store(false, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let e = entry_for(&url, &body);
    let http = reqwest::Client::new();
    let _ = download(&http, &e, dir.path(), plenty, |_, _| {}).await;
    let path = download(&http, &e, dir.path(), plenty, |_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
}

#[tokio::test]
async fn a_wrong_checksum_deletes_the_file() {
    let body = model_bytes();
    let (url, _) = server(body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let mut e = entry_for(&url, &body);
    e.sha256 = "0".repeat(64);
    let r = download(&reqwest::Client::new(), &e, dir.path(), plenty, |_, _| {}).await;
    assert_eq!(r, Err(DownloadError::Checksum));
    assert!(!dir.path().join("dl-model.gguf").exists());
    assert!(!host::library::part_path(dir.path(), &e).exists());
}

#[tokio::test]
async fn not_enough_disk_space_is_said_before_downloading() {
    let body = model_bytes();
    let (url, srv) = server(body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let e = entry_for(&url, &body);
    let r = download(&reqwest::Client::new(), &e, dir.path(), |_| Some(10 * 1024 * 1024), |_, _| {}).await;
    assert!(matches!(r, Err(DownloadError::NoSpace { .. })), "{r:?}");
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0, "nothing was requested");
}

#[tokio::test]
async fn adding_from_the_models_screen_installs_reads_the_shape_and_offers_it() {
    let body = model_bytes();
    let (url, _) = server(body.clone()).await;
    let e = entry_for(&url, &body);
    let host = start_host_with(&[], std::path::PathBuf::from(env!("CARGO_BIN_EXE_fake-llama-server")), vec![entry("model-a")], vec![]).await;
    host.core.set_catalog(vec![entry("model-a"), e.clone()]);
    assert!(!host.core.installed().iter().any(|i| i.id == "dl-model"));

    host::library::start_download(host.core.clone(), "dl-model".into());
    for _ in 0..200 {
        if host.core.installed().iter().any(|i| i.id == "dl-model") && !host.core.downloading("dl-model") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let installed = host.core.installed();
    let got = installed.iter().find(|i| i.id == "dl-model").expect("installed");
    assert_eq!(got.shape, Some(protocol::models::ModelShape { layers: 24, kv_heads: 4, head_dim: 128 }), "shape read from the file");
    assert!(host.core.offered().iter().any(|o| o.id == "dl-model"));

    // and Remove deletes it
    host::library::remove(&host.core, "dl-model").await.unwrap();
    assert!(!host.core.installed().iter().any(|i| i.id == "dl-model"));
}

#[tokio::test]
async fn the_default_model_comes_first_for_friends() {
    let host = start_host(&[]).await;
    assert_eq!(host.core.offered()[0].id, "model-a");
    host.core.update_settings(|s| s.default_model = Some("model-b".into()));
    assert_eq!(host.core.offered()[0].id, "model-b");
    let me = host.client(true);
    assert_eq!(me.api.models().await.unwrap().data[0].id, "model-b");
}

#[tokio::test]
#[ignore = "downloads 105 MB from huggingface.co"]
async fn a_real_model_downloads_from_hugging_face() {
    let e = host::library::entry_from_hf_link(
        "https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/resolve/main/SmolLM2-135M-Instruct-Q4_K_M.gguf",
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = download(&reqwest::Client::new(), &e, dir.path(), host::library::free_space, |_, _| {}).await.unwrap();
    let h = host::gguf::read_file(&path).unwrap();
    assert_eq!(h.architecture, "llama");
}
