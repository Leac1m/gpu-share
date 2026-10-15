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

#[tokio::test]
async fn first_run_engine_setup_downloads_verifies_and_unpacks() {
    use std::io::Write;
    let make = |files: &[(&str, &[u8])]| {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        buf.into_inner()
    };
    let server_zip = make(&[("llama-server.exe", b"server"), ("llama-server", b"server"), ("ggml-cuda.dll", b"ggml")]);
    let cudart_zip = make(&[("cudart64_12.dll", b"cudart"), ("cublas64_12.dll", b"cublas")]);
    let (u1, _) = server(server_zip.clone()).await;
    let (u2, _) = server(cudart_zip.clone()).await;
    let file = |url: String, body: &[u8]| host::engine_setup::ManifestFile { url, sha256: hex::encode(Sha256::digest(body)), size: body.len() as u64 };
    let manifest = host::engine_setup::Manifest { tag: "b1".into(), files: vec![file(u1, &server_zip), file(u2, &cudart_zip)] };
    let dir = tempfile::tempdir().unwrap();
    let engine = host::engine_setup::engine_dir(dir.path(), "b1");
    let last = Mutex::new((0, 0));
    let bin = host::engine_setup::ensure(&reqwest::Client::new(), &manifest, &engine, plenty, |d, t| *last.lock().unwrap() = (d, t))
        .await
        .unwrap();
    assert_eq!(bin, engine.join(host::engine_setup::binary_name()));
    assert!(engine.join("cudart64_12.dll").is_file(), "the CUDA runtime sits next to the server");
    assert!(!engine.join("downloads").exists(), "the archives are cleaned up");
    let (d, t) = *last.lock().unwrap();
    assert_eq!((d, t), (manifest.total_bytes(), manifest.total_bytes()));
    // a second start finds it ready without downloading
    assert!(host::engine_setup::ensure(&reqwest::Client::new(), &manifest, &engine, plenty, |_, _| panic!("downloaded again")).await.is_ok());
}

#[tokio::test]
async fn the_linux_engine_waits_for_the_first_switch_on_then_unpacks_the_tar_gz() {
    // a .tar.gz shaped like llama.cpp's Linux builds: a top folder, a library behind a symlink
    let tgz = {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        for (name, data, mode) in [("llama-b1/llama-server", &b"server"[..], 0o755), ("llama-b1/libggml.so.0", b"lib", 0o644)] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            t.append_data(&mut h, name, data).unwrap();
        }
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        t.append_link(&mut h, "llama-b1/libggml.so", "libggml.so.0").unwrap();
        t.into_inner().unwrap().finish().unwrap()
    };
    let (url, srv) = server(tgz.clone()).await;
    let manifest = host::engine_setup::Manifest {
        tag: "b1".into(),
        files: vec![host::engine_setup::ManifestFile {
            url: format!("{url}?name=llama-b1-bin-ubuntu-vulkan-x64.tar.gz"),
            sha256: hex::encode(Sha256::digest(&tgz)),
            size: tgz.len() as u64,
        }],
    };
    let h = start_host(&[]).await;
    h.core.set_sharing(false);
    let dir = host::engine_setup::engine_dir(h.dir.path(), "b1");
    tokio::spawn(host::engine_setup::drive(h.core.clone(), manifest, dir.clone(), host::engine_setup::When::FirstShare));

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0, "nothing is downloaded on a computer that only uses");
    assert!(!h.core.engine_ready(), "but sharing can't start without it");
    assert!(h.core.view().engine_setup.is_none(), "and there's no progress bar yet");

    h.core.set_sharing(true);
    for _ in 0..100 {
        if h.core.engine_ready() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(h.core.engine_ready());
    assert_eq!(srv.hits.load(Ordering::SeqCst), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("llama-server");
        assert_ne!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o111, 0, "executable");
        assert_eq!(std::fs::read_link(dir.join("libggml.so")).unwrap(), std::path::PathBuf::from("libggml.so.0"));
    }
    assert!(!dir.with_file_name("b1.partial").exists() && !dir.with_file_name("b1.downloads").exists(), "no leftovers");
}

#[tokio::test]
async fn a_failed_engine_setup_is_retried_by_switching_sharing_off_and_on() {
    let body = b"not the archive we expected".to_vec();
    let (url, srv) = server(body.clone()).await;
    let manifest = host::engine_setup::Manifest {
        tag: "b2".into(),
        files: vec![host::engine_setup::ManifestFile { url: format!("{url}?name=x.tar.gz"), sha256: "0".repeat(64), size: body.len() as u64 }],
    };
    let h = start_host(&[]).await;
    let dir = host::engine_setup::engine_dir(h.dir.path(), "b2");
    tokio::spawn(host::engine_setup::drive(h.core.clone(), manifest, dir, host::engine_setup::When::Now));
    let stage = || h.core.view().engine_setup.map(|d| d.stage).unwrap_or_default();
    for _ in 0..100 {
        if stage().contains("Switch sharing off and on") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(stage().contains("Switch sharing off and on"), "{}", stage());
    let hits = srv.hits.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(srv.hits.load(Ordering::SeqCst), hits, "no retry loop on its own");
    h.core.set_sharing(false);
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.core.set_sharing(true);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(srv.hits.load(Ordering::SeqCst) > hits, "switching back on tries again");
    assert!(!h.core.engine_ready());
}

/// The real Linux engine from the pinned llama.cpp release, through the app's own download,
/// checksum and unpacking code, then started (`P8-T1`). Run with `-- --ignored`.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "downloads 30 MB from github.com"]
async fn the_real_linux_engine_downloads_unpacks_and_runs() {
    let tag = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../engine-version.txt")).unwrap().trim().to_string();
    let url = format!("https://github.com/ggml-org/llama.cpp/releases/download/{tag}/llama-{tag}-bin-ubuntu-vulkan-x64.tar.gz");
    let http = reqwest::Client::new();
    // the checksum the release workflow would put in the manifest
    let body = http.get(&url).send().await.unwrap().error_for_status().unwrap().bytes().await.unwrap();
    let manifest = host::engine_setup::Manifest {
        tag: tag.clone(),
        files: vec![host::engine_setup::ManifestFile { url, sha256: hex::encode(Sha256::digest(&body)), size: body.len() as u64 }],
    };
    let dir = tempfile::tempdir().unwrap();
    let engine = host::engine_setup::engine_dir(dir.path(), &tag);
    let bin = host::engine_setup::ensure(&http, &manifest, &engine, plenty, |_, _| {}).await.unwrap();
    let out = std::process::Command::new(&bin).args(["--device", "none", "--version"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("version"), "{text}");
}
