use crate::paths::models_dir;
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Archive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub ready: bool,
    pub path: String,
    pub download_url: Option<String>,
    pub size_hint_bytes: Option<u64>,
    /// Expected SHA-256 of the artifact served by `download_url`.
    /// Taken from the Hugging Face LFS oid, which is the file digest.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
    pub phase: String,
}

/// Catalog of local models with **direct** artifact URLs (not empty placeholders).
///
/// This is the only source of download URLs. Nothing reaching the IPC boundary can
/// point the downloader somewhere else: a model artifact is parsed by whisper.cpp and
/// llama.cpp, so an attacker-chosen file is an attacker-chosen input to a C++ parser.
pub fn list_models() -> Vec<ModelInfo> {
    let catalog = [
        (
            "whisper-tiny",
            "stt",
            "Whisper Tiny (local STT, ggml)",
            // Official ggerganov whisper.cpp release asset (direct ggml binary)
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            77_691_713u64,
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        ),
        (
            "whisper-base",
            "stt",
            "Whisper Base (local STT, ggml)",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            147_951_465u64,
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        ),
        (
            "qwen2.5-0.5b",
            "llm",
            "Qwen2.5 0.5B Instruct Q4_K_M (local LLM)",
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
            491_400_032u64,
            "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
        ),
        (
            "qwen2.5-1.5b",
            "llm",
            "Qwen2.5 1.5B Instruct Q4_K_M (local LLM)",
            "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
            1_117_320_736u64,
            "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        ),
    ];
    catalog
        .into_iter()
        .map(|(id, kind, label, url, size, sha256)| {
            let path = model_artifact_path(id, kind);
            let ready = path.is_file()
                && std::fs::metadata(&path)
                    .map(|m| m.len() > 1_000_000)
                    .unwrap_or(false);
            ModelInfo {
                id: id.into(),
                kind: kind.into(),
                label: label.into(),
                ready,
                path: path.display().to_string(),
                download_url: Some(url.into()),
                size_hint_bytes: Some(size),
                sha256: sha256.into(),
            }
        })
        .collect()
}

fn model_artifact_path(id: &str, kind: &str) -> PathBuf {
    let name = if kind == "stt" {
        "model.bin"
    } else {
        "model.gguf"
    };
    models_dir().join(id).join(name)
}

/// Download a model with streaming progress callbacks.
/// Handles raw binaries and `.tar` / `.tar.gz` archives.
///
/// The URL is resolved from the internal catalog by `model_id` — it is deliberately
/// not a parameter, so no caller can redirect the download.
pub async fn download_model_with_progress<F>(
    model_id: &str,
    mut on_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(DownloadProgress) + Send,
{
    let info = list_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("unknown model {model_id}"))?;
    let url = info
        .download_url
        .clone()
        .ok_or_else(|| format!("model {model_id} has no download URL in the catalog"))?;
    let url = url.as_str();
    let dest = PathBuf::from(&info.path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    on_progress(DownloadProgress {
        model_id: model_id.into(),
        downloaded_bytes: 0,
        total_bytes: info.size_hint_bytes,
        done: false,
        error: None,
        phase: "connecting".into(),
    });

    let client = reqwest::Client::builder()
        .user_agent("Vesper/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let status = res.status();
        return Err(format!("download failed: {status}"));
    }
    let total = res.content_length().or(info.size_hint_bytes);
    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut downloaded = 0u64;
    let mut stream = res.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        on_progress(DownloadProgress {
            model_id: model_id.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            done: false,
            error: None,
            phase: "downloading".into(),
        });
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    // Verify what came off the network, before anything reads or extracts it.
    on_progress(DownloadProgress {
        model_id: model_id.into(),
        downloaded_bytes: downloaded,
        total_bytes: total,
        done: false,
        error: None,
        phase: "verifying".into(),
    });
    let actual = sha256_file(&tmp)?;
    if !actual.eq_ignore_ascii_case(&info.sha256) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "checksum mismatch for {model_id}: expected {}, got {actual} — artifact discarded",
            info.sha256
        ));
    }

    // Archive handling
    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        on_progress(DownloadProgress {
            model_id: model_id.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            done: false,
            error: None,
            phase: "extracting".into(),
        });
        extract_tar_gz(&tmp, dest.parent().unwrap(), &dest)?;
        let _ = std::fs::remove_file(&tmp);
    } else if lower.ends_with(".tar") {
        on_progress(DownloadProgress {
            model_id: model_id.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            done: false,
            error: None,
            phase: "extracting".into(),
        });
        extract_tar(&tmp, dest.parent().unwrap(), &dest)?;
        let _ = std::fs::remove_file(&tmp);
    } else {
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    }

    let final_len = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if final_len < 1_000_000 {
        let _ = std::fs::remove_file(&dest);
        return Err(format!(
            "downloaded artifact too small ({final_len} bytes) — URL may not be a model file"
        ));
    }

    on_progress(DownloadProgress {
        model_id: model_id.into(),
        downloaded_bytes: final_len,
        total_bytes: Some(final_len),
        done: true,
        error: None,
        phase: "done".into(),
    });
    Ok(dest)
}

/// Streaming SHA-256 of a file — models are hundreds of MB, so never read one whole.
fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hex::encode(hasher.finalize()))
}

fn extract_tar_gz(archive_path: &Path, _out_dir: &Path, dest_file: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);
    extract_first_model_entry(&mut archive, dest_file)
}

fn extract_tar(archive_path: &Path, _out_dir: &Path, dest_file: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let mut archive = Archive::new(file);
    extract_first_model_entry(&mut archive, dest_file)
}

fn extract_first_model_entry<R: std::io::Read>(
    archive: &mut Archive<R>,
    dest_file: &Path,
) -> Result<(), String> {
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?;
        let name = path.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".bin") || name.ends_with(".gguf") || name.ends_with(".onnx") {
            let mut out = std::fs::File::create(dest_file).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err("archive did not contain a .bin/.gguf model file".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn catalog_lists_stt_and_llm_with_direct_urls() {
        let models = list_models();
        assert!(models.iter().any(|m| m.kind == "stt"));
        assert!(models.iter().any(|m| m.kind == "llm"));
        for m in &models {
            let url = m.download_url.as_deref().unwrap_or("");
            assert!(
                url.starts_with("https://") && !url.ends_with(".tar"),
                "expected direct https model URL, got {url}"
            );
            assert!(url.contains("huggingface.co") || url.contains("github.com"));
        }
    }

    #[test]
    fn every_catalog_entry_declares_a_sha256() {
        for m in list_models() {
            assert_eq!(m.sha256.len(), 64, "{} has a malformed digest", m.id);
            assert!(
                m.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{} digest is not hex: {}",
                m.id,
                m.sha256
            );
            assert!(m.size_hint_bytes.unwrap_or(0) > 1_000_000, "{}", m.id);
        }
    }

    #[test]
    fn sha256_file_matches_known_digest() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("abc.bin");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_comparison_rejects_a_different_artifact() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("tampered.bin");
        std::fs::write(&p, b"not the model you asked for").unwrap();
        let expected = list_models()
            .into_iter()
            .find(|m| m.id == "whisper-tiny")
            .unwrap()
            .sha256;
        assert!(!sha256_file(&p).unwrap().eq_ignore_ascii_case(&expected));
    }

    #[test]
    fn artifact_paths_use_bin_and_gguf() {
        let stt = model_artifact_path("whisper-tiny", "stt");
        assert!(stt.ends_with("model.bin"));
        let llm = model_artifact_path("qwen2.5-0.5b", "llm");
        assert!(llm.ends_with("model.gguf"));
    }
}
