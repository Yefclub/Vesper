use crate::paths::models_dir;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub ready: bool,
    pub path: String,
    pub download_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
}

pub fn list_models() -> Vec<ModelInfo> {
    let catalog = [
        ("moonshine-base", "stt", "Moonshine Base (local STT)"),
        ("parakeet-tdt-0.6b", "stt", "Parakeet TDT 0.6B (local STT)"),
        ("qwen2.5-1.5b", "llm", "Qwen2.5 1.5B (local LLM)"),
        ("gemma-2b", "llm", "Gemma 2B (local LLM)"),
    ];
    catalog
        .into_iter()
        .map(|(id, kind, label)| {
            let path = model_artifact_path(id, kind);
            ModelInfo {
                id: id.into(),
                kind: kind.into(),
                label: label.into(),
                ready: path.is_file(),
                path: path.display().to_string(),
                download_url: default_url(id),
            }
        })
        .collect()
}

fn model_artifact_path(id: &str, kind: &str) -> PathBuf {
    let ext = if kind == "stt" { "model.onnx" } else { "model.gguf" };
    models_dir().join(id).join(ext)
}

fn default_url(id: &str) -> Option<String> {
    // Placeholder catalog URLs — first-run download uses these when user confirms.
    Some(format!(
        "https://github.com/Yefclub/Vesper/releases/download/models/{id}.tar"
    ))
}

/// Download a model file to the local models directory (streaming).
pub async fn download_model(model_id: &str, url: &str) -> Result<PathBuf, String> {
    let info = list_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("unknown model {model_id}"))?;
    let dest = PathBuf::from(&info.path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let client = reqwest::Client::new();
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("download failed: {}", res.status()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_stt_and_llm() {
        let models = list_models();
        assert!(models.iter().any(|m| m.kind == "stt"));
        assert!(models.iter().any(|m| m.kind == "llm"));
    }
}
