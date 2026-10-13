//! Getting the AI engine onto a Windows host on first run (`P7-T3`, spec §6.10): the CUDA build
//! of llama-server and the CUDA runtime are several hundred MB, so the installer carries only a
//! manifest (URLs, sizes, SHA-256s, written by the release workflow from the pinned engine
//! version), and the app downloads and checks the files once.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    let downloads = dir.join("downloads");
    let total = manifest.total_bytes();
    let mut before = 0;
    for (i, f) in manifest.files.iter().enumerate() {
        let spec = FileSpec { url: f.url.clone(), sha256: f.sha256.clone(), size_bytes: f.size, file_name: format!("part{i}.zip") };
        let zip = download_file(http, &spec, &downloads, free, |d, _| progress(before + d, total)).await?;
        unzip_flat(&zip, dir)?;
        before += f.size;
    }
    if !bin.is_file() {
        return Err(SetupError::NoBinary(binary_name()));
    }
    let _ = std::fs::remove_dir_all(&downloads);
    Ok(bin)
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

    #[test]
    fn the_placeholder_manifest_is_empty() {
        let m: Manifest = serde_json::from_str(r#"{"tag":"b11582"}"#).unwrap();
        assert!(m.files.is_empty());
        assert_eq!(m.total_bytes(), 0);
    }
}
