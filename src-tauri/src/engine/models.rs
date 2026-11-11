//! Catalog and downloader for whisper.cpp ggml models.
//!
//! URLs are pinned to a Hugging Face commit and every file is verified
//! against its SHA-256 (the Git LFS object id) before it is moved into
//! place, so a truncated or tampered download can never be loaded.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

const REPO: &str = "ggerganov/whisper.cpp";
/// Pinned commit of the model repo (verified 2026-11-11).
const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const MODELS_DIR: &str = "models";
const COPY_BUFFER_BYTES: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Per-read stall timeout; the whole download may legitimately take long.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: &'static str,
    pub file_name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub recommended: bool,
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "large-v3-turbo",
        file_name: "ggml-large-v3-turbo.bin",
        label: "Whisper large-v3-turbo",
        description: "En iyi hız/doğruluk dengesi. Türkçe için önerilen.",
        size_bytes: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        recommended: true,
    },
    CatalogEntry {
        id: "large-v3-turbo-q8_0",
        file_name: "ggml-large-v3-turbo-q8_0.bin",
        label: "Whisper large-v3-turbo (Q8)",
        description: "8-bit nicemlenmiş; neredeyse aynı doğruluk, yarı boyut.",
        size_bytes: 874_188_075,
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        recommended: false,
    },
    CatalogEntry {
        id: "large-v3-turbo-q5_0",
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        label: "Whisper large-v3-turbo (Q5)",
        description: "En küçük; az bellek, hafif doğruluk kaybı.",
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        recommended: false,
    },
    CatalogEntry {
        id: "large-v3",
        file_name: "ggml-large-v3.bin",
        label: "Whisper large-v3",
        description: "Tam model; en yavaş, zor kayıtlarda biraz daha isabetli.",
        size_bytes: 3_095_033_483,
        sha256: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        recommended: false,
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub installed: bool,
    pub path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("Bilinmeyen model: {0}")]
    UnknownModel(String),
    #[error("İndirme iptal edildi.")]
    Cancelled,
    #[error("Ağ hatası: {0}")]
    Network(String),
    #[error("Dosya hatası: {0}")]
    Io(#[from] std::io::Error),
    #[error("Doğrulama başarısız: dosya bozuk veya değiştirilmiş.")]
    Integrity,
}

pub fn models_dir(app_data: &Path) -> PathBuf {
    app_data.join(MODELS_DIR)
}

pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

fn url_for(entry: &CatalogEntry) -> String {
    format!(
        "https://huggingface.co/{REPO}/resolve/{REVISION}/{}",
        entry.file_name
    )
}

/// Installed means the file exists with the exact expected size; the hash
/// was verified when it was downloaded.
pub fn list(app_data: &Path) -> Vec<ModelStatus> {
    let dir = models_dir(app_data);
    CATALOG
        .iter()
        .map(|entry| {
            let path = dir.join(entry.file_name);
            let installed = fs::metadata(&path)
                .map(|m| m.len() == entry.size_bytes)
                .unwrap_or(false);
            ModelStatus {
                entry: *entry,
                installed,
                path: path.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes chunks to a file while hashing them; yields size and digest.
struct HashingSink<W: Write> {
    writer: W,
    hasher: Sha256,
    total: u64,
}

impl<W: Write> HashingSink<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            hasher: Sha256::new(),
            total: 0,
        }
    }

    fn write(&mut self, chunk: &[u8]) -> Result<(), DownloadError> {
        self.hasher.update(chunk);
        self.writer.write_all(chunk)?;
        self.total += chunk.len() as u64;
        Ok(())
    }

    /// Returns `(bytes_written, lowercase_hex_sha256, writer)`.
    fn finish(mut self) -> Result<(u64, String, W), DownloadError> {
        self.writer.flush()?;
        Ok((self.total, hex(&self.hasher.finalize()), self.writer))
    }
}

fn network(err: reqwest::Error) -> DownloadError {
    DownloadError::Network(err.without_url().to_string())
}

/// Streams the file into `part_path`. Uses the async client because only it
/// offers a per-read stall timeout; a total timeout would kill multi-GB
/// downloads on slow links.
async fn fetch(
    entry: &CatalogEntry,
    part_path: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(u64, String), DownloadError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(network)?;
    let mut response = client
        .get(url_for(entry))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(network)?;
    let mut sink = HashingSink::new(BufWriter::with_capacity(
        COPY_BUFFER_BYTES,
        File::create(part_path)?,
    ));
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        sink.write(&chunk)?;
        on_progress(sink.total, entry.size_bytes);
    }
    let (written, digest, writer) = sink.finish()?;
    writer.get_ref().sync_all()?;
    Ok((written, digest))
}

/// Downloads, verifies and atomically installs one catalog model. Blocking;
/// call it from a worker thread.
pub fn download(
    id: &str,
    app_data: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, DownloadError> {
    let entry = find(id).ok_or_else(|| DownloadError::UnknownModel(id.to_owned()))?;
    let dir = models_dir(app_data);
    fs::create_dir_all(&dir)?;
    let final_path = dir.join(entry.file_name);
    let part_path = dir.join(format!("{}.part", entry.file_name));

    let result = tauri::async_runtime::block_on(fetch(entry, &part_path, cancel, on_progress))
        .and_then(|(written, digest)| {
            if written != entry.size_bytes || digest != entry.sha256 {
                return Err(DownloadError::Integrity);
            }
            fs::rename(&part_path, &final_path)?;
            Ok(final_path.clone())
        });

    if result.is_err() {
        if let Err(err) = fs::remove_file(&part_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%err, "could not remove partial download");
            }
        }
    }
    result
}

/// Deletes an installed catalog model. Only catalog files inside the models
/// directory can be removed; arbitrary paths are never accepted.
pub fn delete(id: &str, app_data: &Path) -> Result<(), DownloadError> {
    let entry = find(id).ok_or_else(|| DownloadError::UnknownModel(id.to_owned()))?;
    match fs::remove_file(models_dir(app_data).join(entry.file_name)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_hashes_are_sha256_hex() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        for entry in CATALOG {
            assert_eq!(entry.sha256.len(), 64);
            assert!(entry
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
        assert_eq!(CATALOG.iter().filter(|e| e.recommended).count(), 1);
    }

    #[test]
    fn url_is_pinned_to_revision() {
        let url = url_for(find("large-v3-turbo").unwrap());
        assert!(url.contains(REVISION));
        assert!(url.ends_with("/ggml-large-v3-turbo.bin"));
    }

    #[test]
    fn hashing_sink_matches_known_digest_across_chunks() {
        let mut sink = HashingSink::new(Vec::new());
        sink.write(b"a").unwrap();
        sink.write(b"bc").unwrap();
        let (n, digest, out) = sink.finish().unwrap();
        assert_eq!((n, out.as_slice()), (3, &b"abc"[..]));
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn list_reports_installed_only_with_exact_size() {
        let dir = tempfile::tempdir().unwrap();
        let models = models_dir(dir.path());
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join("ggml-large-v3-turbo-q5_0.bin"), b"short").unwrap();
        assert!(list(dir.path()).iter().all(|m| !m.installed));
    }

    #[test]
    fn delete_rejects_unknown_ids() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            delete("../../etc/passwd", dir.path()),
            Err(DownloadError::UnknownModel(_))
        ));
        assert!(delete("large-v3", dir.path()).is_ok());
    }
}
