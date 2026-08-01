//! Local light LLM via llama.cpp (`llama_cpp` crate).
//! When GGUF weights exist, runs real generation. Without weights, returns a clear error
//! for summarize/chat cloud-less paths (callers may fall back intentionally).

use crate::domain::chat::{offline_answer, ChatMessage};
use crate::domain::summary::{extractive_summary, MeetingInsights, SummaryTemplate};
use llama_cpp::standard_sampler::StandardSampler;
use llama_cpp::{LlamaModel, LlamaParams, SessionParams};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static LLM_CACHE: OnceLock<Mutex<Option<(String, LlamaModel)>>> = OnceLock::new();

fn llm_cache() -> &'static Mutex<Option<(String, LlamaModel)>> {
    LLM_CACHE.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone)]
pub struct LocalLlm {
    models_dir: PathBuf,
    /// When true (default in production), missing model → extractive offline fallback.
    /// Tests can set false to assert hard errors.
    pub soft_fallback: bool,
}

impl LocalLlm {
    pub fn new() -> Self {
        Self {
            models_dir: crate::paths::models_dir(),
            soft_fallback: true,
        }
    }

    pub fn with_models_dir(dir: PathBuf) -> Self {
        Self {
            models_dir: dir,
            soft_fallback: true,
        }
    }

    pub fn model_file(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id).join("model.gguf")
    }

    pub fn is_ready(&self, model_id: &str) -> bool {
        let p = self.model_file(model_id);
        p.is_file() && std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false)
    }

    pub fn summarize(
        &self,
        transcript: &str,
        template: SummaryTemplate,
        model_id: &str,
    ) -> Result<MeetingInsights, String> {
        if self.is_ready(model_id) {
            let prompt = crate::domain::summary::build_summary_prompt(template, transcript);
            let raw = run_llama(&self.model_file(model_id), &prompt, 512)?;
            return Ok(MeetingInsights::from_model_text(&raw));
        }
        if self.soft_fallback {
            return Ok(extractive_summary(transcript, 4));
        }
        Err(format!(
            "local LLM model `{model_id}` is not installed — download GGUF weights from Settings"
        ))
    }

    pub fn chat(
        &self,
        messages: &[ChatMessage],
        transcript: &str,
        model_id: &str,
    ) -> Result<String, String> {
        let question = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        if self.is_ready(model_id) {
            let system = messages
                .iter()
                .find(|m| m.role == "system")
                .map(|m| m.content.as_str())
                .unwrap_or("You are Vesper, a private meeting assistant.");
            let prompt = format!(
                "{system}\n\nUser: {question}\nAssistant:"
            );
            return run_llama(&self.model_file(model_id), &prompt, 384);
        }
        if self.soft_fallback {
            return Ok(offline_answer(transcript, question));
        }
        Err(format!(
            "local LLM model `{model_id}` is not installed — download GGUF weights from Settings"
        ))
    }
}

impl Default for LocalLlm {
    fn default() -> Self {
        Self::new()
    }
}

/// Real llama.cpp generation — loads GGUF and samples tokens.
pub fn run_llama(model_path: &Path, prompt: &str, max_tokens: usize) -> Result<String, String> {
    if !model_path.is_file() {
        return Err(format!("GGUF model missing: {}", model_path.display()));
    }
    let meta = std::fs::metadata(model_path).map_err(|e| e.to_string())?;
    if meta.len() < 1_000_000 {
        return Err(
            "GGUF model file is too small to be valid weights (download may be incomplete)".into(),
        );
    }

    let key = model_path.display().to_string();
    let mut cache = llm_cache().lock();
    let need_load = cache.as_ref().map(|(k, _)| k != &key).unwrap_or(true);
    if need_load {
        let model = LlamaModel::load_from_file(model_path, LlamaParams::default())
            .map_err(|e| format!("llama load failed: {e}"))?;
        *cache = Some((key, model));
    }
    let model = cache.as_ref().unwrap().1.clone();

    let mut session = model
        .create_session(SessionParams::default())
        .map_err(|e| format!("llama session: {e}"))?;
    session
        .advance_context(prompt.as_bytes())
        .map_err(|e| format!("llama context: {e}"))?;

    let mut completions = session
        .start_completing_with(StandardSampler::default(), max_tokens)
        .map_err(|e| format!("llama complete: {e}"))?
        .into_strings();

    let mut out = String::new();
    let mut n = 0usize;
    for piece in completions.by_ref() {
        out.push_str(&piece);
        n += 1;
        if n >= max_tokens {
            break;
        }
    }
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn soft_fallback_summarize_without_model() {
        let llm = LocalLlm::with_models_dir(tempdir().unwrap().path().to_path_buf());
        let i = llm
            .summarize(
                "Me: we need to ship auth. Others: agreed. TODO write tests.",
                SummaryTemplate::General,
                "qwen2.5-1.5b",
            )
            .unwrap();
        assert!(!i.summary.is_empty());
    }

    #[test]
    fn hard_mode_errors_without_model() {
        let mut llm = LocalLlm::with_models_dir(tempdir().unwrap().path().to_path_buf());
        llm.soft_fallback = false;
        let err = llm
            .summarize("hi", SummaryTemplate::General, "qwen2.5-1.5b")
            .unwrap_err();
        assert!(err.contains("not installed"));
    }

    #[test]
    fn run_llama_rejects_tiny_file() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("model.gguf");
        std::fs::write(&p, b"not-gguf").unwrap();
        let err = run_llama(&p, "Hello", 8).unwrap_err();
        assert!(err.contains("too small") || err.contains("llama") || err.contains("GGUF"));
    }
}
