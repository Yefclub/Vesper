//! Local light LLM via llama.cpp (`llama-cpp-2` bindings).
//! When GGUF weights exist, runs real generation. Without weights, returns a clear error
//! for summarize/chat cloud-less paths (callers may fall back intentionally).

use crate::domain::chat::{offline_answer, ChatMessage};
use crate::domain::i18n::Locale;
use crate::domain::summary::{extractive_summary, MeetingInsights, SummaryTemplate};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use parking_lot::Mutex;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// llama.cpp's global init, done once per process.
///
/// It registers the ggml backends and installs a log handler; calling it twice is
/// an error in the C library, so it lives here rather than beside each load.
static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend, String> {
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|e| format!("llama backend init: {e}")))
        .as_ref()
        .map_err(|e| e.clone())
}

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

    /// Ready means "verified against the catalog digest" — llama.cpp parses this
    /// file, so an unverified GGUF must not reach it.
    pub fn is_ready(&self, model_id: &str) -> bool {
        crate::models::artifact_is_verified(&self.model_file(model_id), model_id)
    }

    pub fn summarize(
        &self,
        transcript: &str,
        template: SummaryTemplate,
        locale: Locale,
        model_id: &str,
    ) -> Result<MeetingInsights, String> {
        if self.is_ready(model_id) {
            let prompt = crate::domain::summary::build_summary_prompt(template, transcript, locale);
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

    /// One short completion for a meeting title.
    ///
    /// No `soft_fallback` branch, unlike the two below: the offline substitutes
    /// are keyword-matched transcript lines, which are a usable stopgap for a
    /// summary and garbage as a name. Without weights this is an error and the
    /// caller keeps the date label.
    pub fn title(&self, prompt: &str, model_id: &str) -> Result<String, String> {
        if !self.is_ready(model_id) {
            return Err(format!(
                "local LLM model `{model_id}` is not installed — download GGUF weights from Settings"
            ));
        }
        run_llama(&self.model_file(model_id), prompt, 32)
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
            let prompt = format!("{system}\n\nUser: {question}\nAssistant:");
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

    let backend = backend()?;
    let key = model_path.display().to_string();
    // The lock is held across generation, not just the load. `LlamaModel` is not
    // `Clone` in these bindings and a context borrows it, which is the honest
    // shape: two generations sharing one model would race inside llama.cpp
    // anyway, and the old code's `.clone()` only hid that.
    let mut cache = llm_cache().lock();
    let need_load = cache.as_ref().map(|(k, _)| k != &key).unwrap_or(true);
    if need_load {
        // Dropped before the new load so two models are never resident at once —
        // on a machine that just about fits one, holding both is the difference
        // between working and being killed.
        *cache = None;
        let model = LlamaModel::load_from_file(backend, model_path, &LlamaModelParams::default())
            .map_err(|e| format!("llama load failed: {e}"))?;
        *cache = Some((key, model));
    }
    let model = &cache.as_ref().expect("just loaded").1;

    let tokens = model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|e| format!("llama tokenize: {e}"))?;

    // Sized to this call, and never past what the weights were trained for: a
    // context larger than `n_ctx_train` is allocated, paid for in memory, and
    // gives worse output than the model's real window.
    let want = tokens.len() + max_tokens + 8;
    let n_ctx = want.min(model.n_ctx_train() as usize).max(64) as u32;
    if tokens.len() >= n_ctx as usize {
        // Refusing beats silently truncating: a summary of the first half of a
        // meeting reads exactly like a summary of the meeting.
        return Err(format!(
            "the prompt is {} tokens and this model holds {}",
            tokens.len(),
            n_ctx
        ));
    }

    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(n_ctx));
    let mut ctx = model
        .new_context(backend, ctx_params)
        .map_err(|e| format!("llama context: {e}"))?;

    let mut batch = LlamaBatch::new(n_ctx as usize, 1);
    let last = tokens.len() - 1;
    for (i, token) in tokens.iter().enumerate() {
        // Logits only for the final token: the ones before it are context, and
        // asking for all of them allocates a vocabulary-sized row per token.
        batch
            .add(*token, i as i32, &[0], i == last)
            .map_err(|e| format!("llama batch: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("llama decode: {e}"))?;

    // Deterministic on purpose. A summary the user re-runs should not come back
    // different, and greedy also keeps a seed out of the equation for tests.
    let mut sampler = LlamaSampler::greedy();

    // Bytes, decoded once at the end. A multi-byte character can span two tokens,
    // so decoding each piece on its own turns every accented word in a Portuguese
    // summary into replacement characters.
    let mut out: Vec<u8> = Vec::new();
    for step in 0..max_tokens {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        if model.is_eog_token(token) {
            break;
        }
        sampler.accept(token);
        out.extend_from_slice(
            &model
                .token_to_piece_bytes(token, 32, false, None)
                .map_err(|e| format!("llama detokenize: {e}"))?,
        );
        batch.clear();
        batch
            .add(token, tokens.len() as i32 + step as i32, &[0], true)
            .map_err(|e| format!("llama batch: {e}"))?;
        ctx.decode(&mut batch)
            .map_err(|e| format!("llama decode: {e}"))?;
    }
    Ok(String::from_utf8_lossy(&out).trim().to_string())
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
                Locale::En,
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
            .summarize("hi", SummaryTemplate::General, Locale::En, "qwen2.5-1.5b")
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
