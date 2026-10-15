//! Getting the AI engine onto a host on first use (`P7-T3`, `P8-T1`, spec §6.10). The engine
//! builds are tens to hundreds of MB, so the installer carries only a manifest (URLs, sizes,
//! SHA-256s, written by the release workflow from the pinned engine version), and the app
//! downloads and checks the files once:
//!
//! * Windows: the CUDA 12 build and the CUDA runtime (`.zip`), fetched at first start when an
//!   NVIDIA card is present.
//! * Linux: the Vulkan build (`.tar.gz`), which also runs on the processor when there is no
//!   usable GPU. Fetched when the owner first switches sharing on, so computers that only ever
//!   use a friend's card never download it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::{HostCore, ViewDownload};
use crate::library::{download_file, DownloadError, FileSpec};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// llama.cpp release tag, e.g. "b11582".
    pub tag: String,
    #[serde(default)]
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl Manifest {
    pub fn load(path: &Path) -> Option<Self> {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Where this tag's engine lives once set up.
pub fn engine_dir(data_dir: &Path, tag: &str) -> PathBuf {
    data_dir.join("engine").join(tag)
}

pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("{0}")]
    Download(#[from] DownloadError),
    #[error("the engine archive is damaged: {0}")]
    Archive(String),
    #[error("the engine archive has no {0}")]
    NoBinary(&'static str),
}

/// Downloads, verifies and unpacks every file in the manifest into `dir`. Already-finished
/// setups return at once. `progress` gets (bytes done, bytes total) across all files.
///
/// Archives are unpacked into `<dir>.partial` and renamed into place only once everything is
/// there, so a half-unpacked engine never looks ready. Downloads resume from `<dir>.downloads`.
pub async fn ensure(
    http: &reqwest::Client,
    manifest: &Manifest,
    dir: &Path,
    free: impl Fn(&Path) -> Option<u64> + Copy,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, SetupError> {
    let bin = dir.join(binary_name());
    if bin.is_file() {
        return Ok(bin);
    }
    let downloads = sibling(dir, "downloads");
    let staging = sibling(dir, "partial");
    let total = manifest.total_bytes();
    let mut archives = vec![];
    let mut before = 0;
    for (i, f) in manifest.files.iter().enumerate() {
        let ext = if is_tar_gz(&f.url) { "tar.gz" } else { "zip" };
        let spec = FileSpec { url: f.url.clone(), sha256: f.sha256.clone(), size_bytes: f.size, file_name: format!("part{i}.{ext}") };
        archives.push(download_file(http, &spec, &downloads, free, |d, _| progress(before + d, total)).await?);
        before += f.size;
    }
    let _ = std::fs::remove_dir_all(&staging);
    for a in &archives {
        unpack_flat(a, &staging)?;
    }
    if !staging.join(binary_name()).is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(SetupError::NoBinary(binary_name()));
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&staging, dir).map_err(|e| SetupError::Archive(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&downloads);
    Ok(bin)
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{suffix}"));
    dir.with_file_name(name)
}

fn is_tar_gz(url: &str) -> bool {
    url.ends_with(".tar.gz") || url.ends_with(".tgz")
}

/// Unpacks a `.zip` or `.tar.gz` (by its file name) flat into `dest`.
pub fn unpack_flat(archive: &Path, dest: &Path) -> Result<usize, SetupError> {
    if is_tar_gz(&archive.to_string_lossy()) {
        untar_flat(archive, dest)
    } else {
        unzip_flat(archive, dest)
    }
}

/// When the engine gets fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Right away (Windows with an NVIDIA card: the owner installed the app to share).
    Now,
    /// The first time the owner switches sharing on (Linux, where most installs only use).
    FirstShare,
}

/// Keeps the host paused until the engine is set up, then downloads it (`when`). Shows progress
/// on the Share screen. If it fails, the owner switches sharing off and on to try again.
pub async fn drive(core: Arc<HostCore>, manifest: Manifest, dir: PathBuf, when: When) {
    core.set_engine_missing();
    let mut wait_for_share = when == When::FirstShare;
    loop {
        if wait_for_share {
            // the first switch-on, or the owner's own first question (`P9-T1`)
            while !core.sharing() && !core.engine_wanted() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        core.set_engine_setup(Some(ViewDownload { pct: 0.0, stage: "Starting…".into() }));
        let total = manifest.total_bytes().max(1) as f64;
        let progress = |d: u64, _t: u64| {
            core.set_engine_setup(Some(ViewDownload {
                pct: d as f64 / total * 100.0,
                stage: format!("{:.0} of {:.0} MB", d as f64 / 1e6, total / 1e6),
            }))
        };
        match ensure(&core.download_http, &manifest, &dir, crate::library::free_space, progress).await {
            Ok(_) => {
                core.set_engine_setup(None);
                core.toast("Ready to share", "The AI engine is set up.");
                return;
            }
            Err(e) => {
                log::warn!("engine setup failed: {e}");
                core.set_engine_setup(Some(ViewDownload {
                    pct: 0.0,
                    stage: format!("Couldn't set up the engine: {e}. Switch sharing off and on to try again."),
                }));
                // try again on the next switch-on (or the owner's next question)
                core.clear_engine_wanted();
                while core.sharing() {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                wait_for_share = true;
            }
        }
    }
}

/// Extracts every file in `zip` straight into `dest`, ignoring folders inside the archive (the
/// llama.cpp archives are flat, and the CUDA runtime DLLs must sit next to the server).
/// Entries with unsafe names (absolute, `..`) are skipped.
pub fn unzip_flat(zip: &Path, dest: &Path) -> Result<usize, SetupError> {
    let f = std::fs::File::open(zip).map_err(|e| SetupError::Archive(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(f).map_err(|e| SetupError::Archive(e.to_string()))?;
    std::fs::create_dir_all(dest).map_err(|e| SetupError::Archive(e.to_string()))?;
    let mut n = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| SetupError::Archive(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name().and_then(|p| p.file_name().map(PathBuf::from)) else {
            log::warn!("skipping unsafe archive entry {:?}", entry.name());
            continue;
        };
        let mut out = std::fs::File::create(dest.join(name)).map_err(|e| SetupError::Archive(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| SetupError::Archive(e.to_string()))?;
        std::io::Write::write_all(&mut out, &buf).map_err(|e| SetupError::Archive(e.to_string()))?;
        n += 1;
    }
    Ok(n)
}

/// Extracts a `.tar.gz` flat into `dest`, like [`unzip_flat`]. The Linux builds keep their
/// shared libraries next to `llama-server` (rpath `$ORIGIN`) and link them through symlinks such
/// as `libllama.so -> libllama.so.0`; those are recreated, but only when they point at a plain
/// file name in the same folder. Executable bits are kept.
pub fn untar_flat(archive: &Path, dest: &Path) -> Result<usize, SetupError> {
    let err = |e: std::io::Error| SetupError::Archive(e.to_string());
    let f = std::fs::File::open(archive).map_err(err)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    std::fs::create_dir_all(dest).map_err(err)?;
    let mut n = 0;
    for entry in tar.entries().map_err(err)? {
        let mut entry = entry.map_err(err)?;
        let path = entry.path().map_err(err)?.into_owned();
        let Some(name) = safe_name(&path) else {
            log::warn!("skipping unsafe archive entry {path:?}");
            continue;
        };
        let out = dest.join(&name);
        let kind = entry.header().entry_type();
        if kind.is_file() {
            let mode = entry.header().mode().unwrap_or(0o644);
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(err)?;
            let _ = std::fs::remove_file(&out);
            std::fs::write(&out, &buf).map_err(err)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode)).map_err(err)?;
            }
            let _ = mode;
            n += 1;
        } else if kind.is_symlink() {
            let target = entry.link_name().map_err(err)?.map(|t| t.into_owned());
            let Some(target) = target.filter(|t| t.components().count() == 1 && safe_name(t).is_some()) else {
                log::warn!("skipping symlink {path:?} that leaves the engine folder");
                continue;
            };
            let _ = std::fs::remove_file(&out);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &out).map_err(err)?;
            #[cfg(not(unix))]
            let _ = target;
            n += 1;
        }
    }
    Ok(n)
}

/// The last component of `p`, if `p` stays inside the archive (no root, no `..`).
fn safe_name(p: &Path) -> Option<PathBuf> {
    use std::path::Component;
    if p.components().any(|c| matches!(c, Component::RootDir | Component::Prefix(_) | Component::ParentDir)) {
        return None;
    }
    let name = p.file_name()?;
    (name != "." && !name.is_empty()).then(|| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn unzips_flat_and_skips_escaping_entries() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("a.zip");
        make_zip(&zip, &[("llama-server.exe", b"exe"), ("lib/ggml-cuda.dll", b"dll"), ("../../evil.dll", b"x")]);
        let out = dir.path().join("engine");
        assert_eq!(unzip_flat(&zip, &out).unwrap(), 2);
        assert!(out.join("llama-server.exe").is_file());
        assert!(out.join("ggml-cuda.dll").is_file(), "flattened next to the server");
        assert!(!dir.path().join("evil.dll").exists());
    }

    #[cfg(unix)]
    /// Writes names straight into the headers, so the test archive can hold the hostile paths
    /// (`..`, absolute links) that `tar::Builder` refuses to write.
    pub fn make_tar_gz(path: &Path, files: &[(&str, &[u8], u32)], links: &[(&str, &str)]) {
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(path).unwrap(), flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        let raw = |field: &mut [u8], v: &str| field[..v.len()].copy_from_slice(v.as_bytes());
        for (name, data, mode) in files {
            let mut h = tar::Header::new_gnu();
            raw(&mut h.as_gnu_mut().unwrap().name, name);
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            t.append(&h, *data).unwrap();
        }
        for (name, target) in links {
            let mut h = tar::Header::new_gnu();
            raw(&mut h.as_gnu_mut().unwrap().name, name);
            raw(&mut h.as_gnu_mut().unwrap().linkname, target);
            h.set_size(0);
            h.set_mode(0o777);
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_cksum();
            t.append(&h, std::io::empty()).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn untars_flat_keeps_exec_bits_and_safe_symlinks_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tgz = dir.path().join("a.tar.gz");
        make_tar_gz(
            &tgz,
            &[("llama-b1/llama-server", b"#!/bin/sh\n", 0o755), ("llama-b1/libllama.so.0", b"lib", 0o644), ("llama-b1/../../evil", b"x", 0o755)],
            &[("llama-b1/libllama.so", "libllama.so.0"), ("llama-b1/sneaky", "/etc/passwd"), ("llama-b1/up", "../../x")],
        );
        let out = dir.path().join("engine");
        assert_eq!(untar_flat(&tgz, &out).unwrap(), 3);
        let mode = std::fs::metadata(out.join("llama-server")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the server stays executable");
        assert_eq!(std::fs::metadata(out.join("libllama.so.0")).unwrap().permissions().mode() & 0o111, 0);
        assert_eq!(std::fs::read_link(out.join("libllama.so")).unwrap(), PathBuf::from("libllama.so.0"));
        assert!(!out.join("sneaky").exists() && std::fs::symlink_metadata(out.join("sneaky")).is_err());
        assert!(std::fs::symlink_metadata(out.join("up")).is_err());
        assert!(!dir.path().join("evil").exists());
    }

    #[test]
    fn the_placeholder_manifest_is_empty() {
        let m: Manifest = serde_json::from_str(r#"{"tag":"b11582"}"#).unwrap();
        assert!(m.files.is_empty());
        assert_eq!(m.total_bytes(), 0);
    }
}
