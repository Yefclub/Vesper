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
pub fn list_models() -> Vec<ModelInfo> {
    let catalog = [
        (
            "whisper-tiny",
            "stt",
            "Whisper Tiny (local STT, ggml)",
            // Official ggerganov whisper.cpp release asset (direct ggml binary)
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            Some(77_000_000u64),
        ),
        (
            "whisper-base",
            "stt",
            "Whisper Base (local STT, ggml)",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            Some(148_000_000u64),
        ),
        (
            "qwen2.5-0.5b",
            "llm",
            "Qwen2.5 0.5B Instruct Q4_K_M (local LLM)",
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
            Some(400_000_000u64),
        ),
        (
            "qwen2.5-1.5b",
            "llm",
            "Qwen2.5 1.5B Instruct Q4_K_M (local LLM)",
            "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
            Some(1_000_000_000u64),
        ),
    ];
    catalog
        .into_iter()
        .map(|(id, kind, label, url, size)| {
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
                size_hint_bytes: size,
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
pub async fn download_model_with_progress<F>(
    model_id: &str,
    url: &str,
    mut on_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(DownloadProgress) + Send,
{
    let info = list_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("unknown model {model_id}"))?;
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

pub async fn download_model(model_id: &str, url: &str) -> Result<PathBuf, String> {
    download_model_with_progress(model_id, url, |_| {}).await
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
    fn artifact_paths_use_bin_and_gguf() {
        let stt = model_artifact_path("whisper-tiny", "stt");
        assert!(stt.ends_with("model.bin"));
        let llm = model_artifact_path("qwen2.5-0.5b", "llm");
        assert!(llm.ends_with("model.gguf"));
    }
}
