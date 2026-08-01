//! Local STT via whisper.cpp (`whisper-rs`).
//! When a GGML/GGUF whisper model is present, runs real ASR.
//! Without a model file, returns a clear error (no silent energy-theater as "ASR").

use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

static WHISPER_CACHE: OnceLock<Mutex<Option<(String, WhisperContext)>>> = OnceLock::new();

fn whisper_cache() -> &'static Mutex<Option<(String, WhisperContext)>> {
    WHISPER_CACHE.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone)]
pub struct LocalSttEngine {
    models_dir: PathBuf,
}

impl LocalSttEngine {
    pub fn new() -> Self {
        Self {
            models_dir: crate::paths::models_dir(),
        }
    }

    pub fn with_models_dir(models_dir: PathBuf) -> Self {
        Self { models_dir }
    }

    /// Resolve model artifact path. Accepts either `model.bin` (whisper ggml) or legacy `model.onnx`.
    pub fn model_path(&self, model_id: &str) -> PathBuf {
        let dir = self.models_dir.join(model_id);
        let bin = dir.join("model.bin");
        if bin.is_file() {
            return bin;
        }
        let ggml = dir.join("ggml-model.bin");
        if ggml.is_file() {
            return ggml;
        }
        // default expected download target
        bin
    }

    /// Ready means "verified against the catalog digest", not merely "a big file is
    /// there". whisper.cpp parses this file, so bytes of unknown provenance must not
    /// reach it just because they predate the checksum work.
    pub fn is_model_ready(&self, model_id: &str) -> bool {
        crate::models::artifact_is_verified(&self.model_path(model_id), model_id)
    }

    /// Transcribe mono PCM with local Whisper when model weights exist.
    pub fn transcribe(
        &self,
        pcm: &[i16],
        sample_rate: u32,
        model_id: &str,
        language: &str,
    ) -> Result<String, String> {
        if pcm.is_empty() {
            return Ok(String::new());
        }
        let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        if !self.is_model_ready(model_id) {
            // Silent frames: no-op. Voiced frames without weights: hard error (not fake ASR).
            if peak < 400 {
                return Ok(String::new());
            }
            return Err(format!(
                "local STT model `{model_id}` is not installed — download Whisper GGML from Settings"
            ));
        }
        let path = self.model_path(model_id);
        run_whisper(&path, pcm, sample_rate, language)
    }
}

impl Default for LocalSttEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Real whisper.cpp inference path — always uses WhisperContext when called.
pub fn run_whisper(
    model_path: &Path,
    pcm: &[i16],
    sample_rate: u32,
    language: &str,
) -> Result<String, String> {
    if !model_path.is_file() {
        return Err(format!("whisper model missing: {}", model_path.display()));
    }
    let meta = std::fs::metadata(model_path).map_err(|e| e.to_string())?;
    if meta.len() < 1_000_000 {
        return Err(
            "whisper model file is too small to be a real GGML model (download may be incomplete)"
                .into(),
        );
    }

    let key = model_path.display().to_string();
    let mut cache = whisper_cache().lock();
    let need_load = cache
        .as_ref()
        .map(|(k, _)| k != &key)
        .unwrap_or(true);
    if need_load {
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or("non-utf8 model path")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("whisper load failed: {e}"))?;
        *cache = Some((key, ctx));
    }
    let ctx = &cache.as_ref().unwrap().1;

    // Resample to 16 kHz mono f32 expected by whisper
    let audio = resample_to_16k_f32(pcm, sample_rate);

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("whisper state: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    if language != "auto" && !language.is_empty() {
        params.set_language(Some(language));
    } else {
        params.set_language(Some("auto"));
    }
    params.set_n_threads(num_cpus_soft());

    state
        .full(params, &audio)
        .map_err(|e| format!("whisper inference failed: {e}"))?;

    let mut out = String::new();
    for segment in state.as_iter() {
        let text = segment.to_string();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(text);
    }
    Ok(out.trim().to_string())
}

fn num_cpus_soft() -> i32 {
    std::thread::available_parallelism()
        .map(|n| (n.get() as i32).clamp(1, 8))
        .unwrap_or(4)
}

/// Linear resample i16 PCM → 16 kHz f32 mono in [-1, 1].
pub fn resample_to_16k_f32(pcm: &[i16], sample_rate: u32) -> Vec<f32> {
    if pcm.is_empty() {
        return Vec::new();
    }
    let sr = sample_rate.max(1) as f64;
    let target = 16_000f64;
    if (sr - target).abs() < 1.0 {
        return pcm
            .iter()
            .map(|s| *s as f32 / i16::MAX as f32)
            .collect();
    }
    let ratio = target / sr;
    let out_len = ((pcm.len() as f64) * ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(pcm.len() - 1);
        let frac = (src - i0 as f64) as f32;
        let a = pcm[i0] as f32 / i16::MAX as f32;
        let b = pcm[i1] as f32 / i16::MAX as f32;
        out.push(a * (1.0 - frac) + b * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resample_identity_at_16k() {
        let pcm = vec![0i16, 1000, -1000];
        let f = resample_to_16k_f32(&pcm, 16_000);
        assert_eq!(f.len(), 3);
        assert!((f[1] - 1000.0 / i16::MAX as f32).abs() < 1e-4);
    }

    #[test]
    fn missing_model_errors_honestly() {
        let dir = tempdir().unwrap();
        let engine = LocalSttEngine::with_models_dir(dir.path().to_path_buf());
        let err = engine
            .transcribe(&[3000i16; 1600], 16_000, "whisper-tiny", "en")
            .unwrap_err();
        assert!(err.contains("not installed") || err.contains("download"));
    }

    #[test]
    fn tiny_fake_file_rejected_by_whisper_path() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-tiny");
        std::fs::create_dir_all(&model_dir).unwrap();
        let p = model_dir.join("model.bin");
        std::fs::write(&p, b"not-a-real-ggml").unwrap();
        // is_model_ready requires >1MB so this is "not ready"
        let engine = LocalSttEngine::with_models_dir(dir.path().to_path_buf());
        assert!(!engine.is_model_ready("whisper-tiny"));
        // Direct run_whisper must fail for junk file (proves real engine entry)
        let err = run_whisper(&p, &[1000i16; 1600], 16_000, "en").unwrap_err();
        assert!(
            err.contains("too small") || err.contains("whisper"),
            "unexpected: {err}"
        );
    }
}
