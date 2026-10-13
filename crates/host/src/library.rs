//! The model library (`P6-T1`–`P6-T4`, `P6-T7`, `P6-T8`, spec §6.5): the catalog, downloads, and
//! adding or removing models.
//!
//! * Catalog: the bundled `catalog.json`, replaced by a fetched copy when one is configured
//!   (`GPU_SHARE_CATALOG_URL`) and cached for offline starts. Every entry is validated: the id
//!   becomes a file name, so it must be a plain slug, and downloads only come from allowed hosts.
//! * Downloads are resumable (HTTP Range onto a `.part` file), check free disk space first, hash
//!   as they go, and only become a model once the SHA-256 matches. Files land only in the app's
//!   own models folder.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use protocol::models::CatalogEntry;
use sha2::{Digest, Sha256};

use crate::core::{HostCore, ViewDownload};

/// Room to leave on the disk on top of the model itself.
const DISK_MARGIN: u64 = 1024 * 1024 * 1024;

/// A catalog id is used as a file name: lowercase letters, digits, `.`, `-`, `_`, no leading dot.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        && !id.contains("..")
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    Some(rest.split(['/', ':']).next().unwrap_or(""))
}

/// Entries that are safe to use: valid id, 64-hex SHA-256, a size, and a download URL on an
/// allowed host. HTTPS only, except `127.0.0.1` (tests' local server) when it is allowed.
pub fn validate(entries: Vec<CatalogEntry>, allowed_hosts: &[String]) -> Vec<CatalogEntry> {
    entries
        .into_iter()
        .filter(|e| {
            let ok_url = host_of(&e.file_url).is_some_and(|h| {
                allowed_hosts.iter().any(|a| a == h) && (e.file_url.starts_with("https://") || h == "127.0.0.1")
            });
            let ok = valid_id(&e.id)
                && e.sha256.len() == 64
                && e.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && e.size_bytes > 0
                && ok_url;
            if !ok {
                log::warn!("ignoring catalog entry {:?}", e.id);
            }
            ok
        })
        .collect()
}

pub const DEFAULT_HOSTS: &[&str] = &["huggingface.co"];

/// The catalog to use at startup: fetched (and cached) if a URL is configured, else the cache
/// from a previous fetch, else the bundled one. Dev entries from the bundled catalog are kept.
pub async fn load_catalog(bundled: Vec<CatalogEntry>, url: Option<&str>, cache: &Path, hosts: &[String]) -> Vec<CatalogEntry> {
    let dev: Vec<CatalogEntry> = bundled.iter().filter(|e| e.dev_only).cloned().collect();
    let fetched = match url {
        Some(url) => fetch_catalog(url).await,
        None => None,
    };
    let remote = match fetched {
        Some(list) => {
            let _ = std::fs::write(cache, serde_json::to_vec_pretty(&list).unwrap());
            Some(list)
        }
        None if url.is_some() => std::fs::read(cache).ok().and_then(|b| serde_json::from_slice(&b).ok()),
        None => None,
    };
    let mut list = validate(remote.unwrap_or(bundled), hosts);
    for d in dev {
        if !list.iter().any(|e| e.id == d.id) {
            list.push(d);
        }
    }
    list
}

async fn fetch_catalog(url: &str) -> Option<Vec<CatalogEntry>> {
    let r = reqwest::Client::new().get(url).timeout(Duration::from_secs(10)).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

/// Free space on the disk holding `dir`, in bytes.
pub fn free_space(dir: &Path) -> Option<u64> {
    let dir = dir.canonicalize().ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DownloadError {
    #[error("not enough disk space: {need_gb:.1} GB needed, {free_gb:.1} GB free")]
    NoSpace { need_gb: f64, free_gb: f64 },
    #[error("the download failed: {0}")]
    Network(String),
    #[error("the file didn't match its checksum, so it was deleted")]
    Checksum,
    #[error("could not write the file: {0}")]
    Disk(String),
}

pub fn part_path(dir: &Path, e: &CatalogEntry) -> PathBuf {
    dir.join(format!("{}.part", e.file_name()))
}

/// One file to fetch and verify.
#[derive(Debug, Clone)]
pub struct FileSpec {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    /// Final name inside the destination folder.
    pub file_name: String,
}

impl From<&CatalogEntry> for FileSpec {
    fn from(e: &CatalogEntry) -> Self {
        Self { url: e.file_url.clone(), sha256: e.sha256.clone(), size_bytes: e.size_bytes, file_name: e.file_name() }
    }
}

/// Downloads a catalog model into `models_dir`. See [`download_file`].
pub async fn download(
    http: &reqwest::Client,
    e: &CatalogEntry,
    models_dir: &Path,
    free: impl Fn(&Path) -> Option<u64>,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, DownloadError> {
    download_file(http, &FileSpec::from(e), models_dir, free, progress).await
}

/// Downloads `f` into `dir`, resuming a `.part` file if there is one. `progress` gets
/// (bytes done, bytes total). `free` reports free disk space (injected for tests).
pub async fn download_file(
    http: &reqwest::Client,
    f: &FileSpec,
    dir: &Path,
    free: impl Fn(&Path) -> Option<u64>,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, DownloadError> {
    std::fs::create_dir_all(dir).map_err(|err| DownloadError::Disk(err.to_string()))?;
    let part = dir.join(format!("{}.part", f.file_name));
    let fin = dir.join(&f.file_name);
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0).min(f.size_bytes);
    let need = f.size_bytes - have;
    if let Some(free) = free(dir) {
        if free < need + DISK_MARGIN {
            return Err(DownloadError::NoSpace {
                need_gb: (need + DISK_MARGIN) as f64 / 1e9,
                free_gb: free as f64 / 1e9,
            });
        }
    }
    // hash what is already there, so resuming still checks the whole file
    let mut hasher = Sha256::new();
    if have > 0 {
        let mut f = std::fs::File::open(&part).map_err(|err| DownloadError::Disk(err.to_string()))?;
        std::io::copy(&mut (&mut f).take(have), &mut HashWriter(&mut hasher)).map_err(|err| DownloadError::Disk(err.to_string()))?;
    }
    let mut done = have;
    let mut req = http.get(&f.url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await.map_err(|err| DownloadError::Network(err.to_string()))?;
    let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !resp.status().is_success() {
        return Err(DownloadError::Network(format!("HTTP {}", resp.status())));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .map_err(|err| DownloadError::Disk(err.to_string()))?;
    if !resumed && have > 0 {
        // the server ignored the range: start over
        hasher = Sha256::new();
        done = 0;
    }
    let mut stream = resp.bytes_stream();
    let mut last = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| DownloadError::Network(err.to_string()))?;
        if done + chunk.len() as u64 > f.size_bytes {
            return Err(DownloadError::Network("the server sent more than expected".into()));
        }
        file.write_all(&chunk).map_err(|err| DownloadError::Disk(err.to_string()))?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(250) {
            progress(done, f.size_bytes);
            last = Instant::now();
        }
    }
    file.flush().map_err(|err| DownloadError::Disk(err.to_string()))?;
    drop(file);
    if done != f.size_bytes {
        return Err(DownloadError::Network(format!("stopped at {done} of {} bytes", f.size_bytes)));
    }
    progress(done, f.size_bytes);
    if hex::encode(hasher.finalize()) != f.sha256.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::Checksum);
    }
    std::fs::rename(&part, &fin).map_err(|err| DownloadError::Disk(err.to_string()))?;
    Ok(fin)
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fmt_mb(b: u64) -> String {
    let mb = b / (1024 * 1024);
    let s = mb.to_string();
    // thousands separators, like the UI elsewhere
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} MB")
}

// ---------------------------------------------------------------------------- the owner's buttons

/// "Add": downloads in the background, showing progress on the Models screen.
pub fn start_download(core: Arc<HostCore>, id: String) {
    let Some(e) = core.catalog().into_iter().find(|e| e.id == id) else { return };
    if core.downloading(&id) || core.installed().iter().any(|i| i.id == id) {
        return;
    }
    core.set_download(&id, Some(ViewDownload { pct: 0.0, stage: "Starting…".into() }));
    tokio::spawn(async move {
        let started = Instant::now();
        let progress = |done: u64, total: u64| {
            let rate = done as f64 / started.elapsed().as_secs_f64().max(0.1) / 1e6;
            let stage = if done < total {
                format!("{} of {} · {rate:.0} MB/s", fmt_mb(done), fmt_mb(total))
            } else {
                "Checking the file…".into()
            };
            core.set_download(&id, Some(ViewDownload { pct: done as f64 / total as f64 * 100.0, stage }));
        };
        let result = download(&core.download_http, &e, &core.cfg.models_dir, free_space, progress).await;
        core.set_download(&id, None);
        match result {
            Ok(path) => {
                core.refresh_shape(&e.id, &path);
                core.toast("Model added", format!("{} is ready and checked.", e.name));
            }
            Err(err) => core.toast("Couldn't add the model", err.to_string()),
        }
    });
}

/// "Remove": unloads the model first if it's loaded, refuses while it's answering someone.
pub async fn remove(core: &HostCore, id: &str) -> Result<(), String> {
    let Some(e) = core.catalog().into_iter().find(|e| e.id == id) else { return Ok(()) };
    if core.supervisor.status().loaded.as_deref() == Some(id) {
        if core.busy() {
            return Err("It's answering a question right now. Try again in a moment.".into());
        }
        core.supervisor.stop().await;
    }
    let path = core.model_path(&e);
    let _ = std::fs::remove_file(part_path(&core.cfg.models_dir, &e));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Couldn't delete the file: {err}")),
    }
}

/// "Add from a Hugging Face link" (`P6-T8`): a `…/resolve/<revision>/<file>.gguf` URL. The size
/// and SHA-256 come from Hugging Face's own headers for the file (its LFS id is the SHA-256), so
/// the download is still verified.
pub async fn entry_from_hf_link(link: &str) -> Result<CatalogEntry, String> {
    // Hugging Face answers with a redirect to its CDN; the checksum and size are on the
    // redirect itself, so don't follow it
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let link = link.trim().replace("/blob/", "/resolve/");
    let rest = link.strip_prefix("https://huggingface.co/").ok_or("Paste a link that starts with https://huggingface.co/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 5 || parts[2] != "resolve" || !parts.last().is_some_and(|f| f.ends_with(".gguf")) {
        return Err("That link doesn't point to a .gguf file. Open the file on Hugging Face and copy its download link.".into());
    }
    let file = parts.last().unwrap().trim_end_matches(".gguf");
    let id: String = format!("hf-{}-{}", parts[0], file)
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' { c } else { '-' })
        .take(64)
        .collect();
    let id = id.replace("..", ".");
    let head = http
        .head(&link)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Hugging Face: {e}"))?;
    if !head.status().is_success() && !head.status().is_redirection() {
        return Err(format!("Hugging Face answered {} for that link.", head.status()));
    }
    let h = head.headers();
    let sha = h
        .get("x-linked-etag")
        .or_else(|| h.get("etag"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_matches('"').trim_start_matches("W/").trim_matches('"').to_string())
        .filter(|s| s.len() == 64)
        .ok_or("Hugging Face didn't give a checksum for that file, so it can't be verified.")?;
    let size = h
        .get("x-linked-size")
        .or_else(|| h.get(reqwest::header::CONTENT_LENGTH))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .ok_or("Hugging Face didn't say how big that file is.")?;
    Ok(CatalogEntry {
        id,
        name: file.replace(['-', '_'], " "),
        description: format!("Added from Hugging Face ({}/{}).", parts[0], parts[1]),
        file_url: link.clone(),
        size_bytes: size,
        sha256: sha,
        quantization: file.rsplit(['-', '.']).next().unwrap_or("").to_string(),
        license: "See the model's page on Hugging Face".into(),
        default_context: 4096,
        shape: None,
        tps_estimate: None,
        engine_args: vec![],
        dev_only: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_safe_file_names() {
        for ok in ["qwen3-8b-q4km", "smollm2-135m-q8", "hf-bartowski-model.q4_k_m"] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["", "../etc", "a/b", "a\\b", ".hidden", "UPPER", "x".repeat(65).as_str(), "a..b", "c:", "con "] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    fn entry(id: &str, url: &str) -> CatalogEntry {
        let mut e: CatalogEntry = serde_json::from_str(include_str!("../../protocol/tests/fixtures/catalog_entry.json")).unwrap();
        e.id = id.into();
        e.file_url = url.into();
        e.sha256 = "a".repeat(64);
        e
    }

    #[test]
    fn catalog_entries_are_validated() {
        let hosts = vec!["huggingface.co".to_string()];
        let kept = validate(
            vec![
                entry("good", "https://huggingface.co/a/b/resolve/main/x.gguf"),
                entry("../evil", "https://huggingface.co/a/b/resolve/main/x.gguf"),
                entry("other-host", "https://example.com/x.gguf"),
                entry("plain-http", "http://huggingface.co/x.gguf"),
                entry("lookalike", "https://huggingface.co.evil.net/x.gguf"),
            ],
            &hosts,
        );
        assert_eq!(kept.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["good"]);
    }

    #[test]
    fn the_bundled_catalog_passes_validation() {
        let all = crate::core::bundled_catalog();
        let n = all.len();
        assert_eq!(validate(all, &["huggingface.co".into()]).len(), n);
    }

    #[test]
    fn megabytes_read_like_the_prototype() {
        assert_eq!(fmt_mb(4_920 * 1024 * 1024), "4,920 MB");
        assert_eq!(fmt_mb(12 * 1024 * 1024), "12 MB");
    }

    #[tokio::test]
    #[ignore = "needs huggingface.co"]
    async fn a_hugging_face_link_becomes_a_verifiable_entry() {
        let e = entry_from_hf_link(
            "https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/blob/main/SmolLM2-135M-Instruct-Q4_K_M.gguf",
        )
        .await
        .unwrap();
        assert_eq!(e.sha256, "2e8040ceae7815abe0dcb3540b9995eaa1fa0d2ca9e797d0a635ae4433c68c2d");
        assert_eq!(e.size_bytes, 105_454_432);
        assert!(valid_id(&e.id), "{}", e.id);
        assert_eq!(validate(vec![e], &["huggingface.co".into()]).len(), 1);
    }

    #[tokio::test]
    async fn links_that_are_not_model_files_are_refused() {
        for bad in ["https://example.com/x.gguf", "https://huggingface.co/org/repo", "https://huggingface.co/org/repo/resolve/main/readme.md"] {
            assert!(entry_from_hf_link(bad).await.is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn an_offline_start_uses_the_cache_then_the_bundled_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("catalog.json");
        let hosts = vec!["huggingface.co".to_string()];
        let bundled = crate::core::bundled_catalog();
        let unreachable = Some("http://127.0.0.1:1/catalog.json");
        let got = load_catalog(bundled.clone(), unreachable, &cache, &hosts).await;
        assert_eq!(got.len(), bundled.len(), "no cache yet: bundled");
        let cached = vec![bundled[0].clone()];
        std::fs::write(&cache, serde_json::to_vec(&cached).unwrap()).unwrap();
        let got = load_catalog(bundled.clone(), unreachable, &cache, &hosts).await;
        let ids: Vec<&str> = got.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["qwen3-4b-q4km", "smollm2-135m-q8"], "the cache, plus the dev model");
    }
}
