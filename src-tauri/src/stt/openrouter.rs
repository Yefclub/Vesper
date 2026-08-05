//! OpenRouter audio transcription client (`/api/v1/audio/transcriptions`).

use crate::domain::cost::usd_to_nano;
use hound::{WavSpec, WavWriter};
use std::io::Cursor;

#[derive(Debug, Clone)]
pub struct OpenRouterStt {
    base_url: String,
    client: reqwest::Client,
}

impl Default for OpenRouterStt {
    fn default() -> Self {
        Self::new()
    }
}

/// Long enough for a slow transcription of a whole meeting, short enough that a
/// silently dropped connection does not hang the recorder forever. Without any
/// timeout at all a stalled request keeps the caller waiting until the process
/// dies.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

impl OpenRouterStt {
    pub fn new() -> Self {
        Self::with_base_url("https://openrouter.ai/api/v1")
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            // Built once and cloned: a fresh Client per request throws away the
            // connection pool, so every chunk paid for a new TLS handshake.
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// `prompt` is the user's vocabulary, already normalised and bounded by
    /// `domain::vocabulary`. Empty for a user who never filled the field in, and
    /// then the field is not sent at all — the request is byte-for-byte the one
    /// this client has always made.
    pub async fn transcribe(
        &self,
        pcm: &[i16],
        sample_rate: u32,
        api_key: &str,
        model: &str,
        language: &str,
        prompt: &str,
    ) -> Result<(String, Option<i64>), String> {
        if api_key.trim().is_empty() {
            return Err("OpenRouter API key required".into());
        }
        let wav = pcm_to_wav_bytes(pcm, sample_rate)?;
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", model.to_string());
        if language != "auto" && !language.is_empty() {
            form = form.text("language", language.to_string());
        }
        if !prompt.is_empty() {
            form = form.text("prompt", prompt.to_string());
        }

        let res = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .header("Authorization", format!("Bearer {api_key}"))
            .header("HTTP-Referer", "https://github.com/Yefclub/Vesper")
            .header("X-Title", "Vesper")
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("OpenRouter STT {status}: {body}"));
        }
        let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        // The transcription endpoint reports its price in the same response, so
        // nothing here has to know what the model charges per second of audio.
        let cost = v["usage"]["cost"].as_f64().and_then(usd_to_nano);
        let text = v
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        Ok((text, cost))
    }
}

pub fn pcm_to_wav_bytes(pcm: &[i16], sample_rate: u32) -> Result<Vec<u8>, String> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = WavWriter::new(&mut cursor, spec).map_err(|e| e.to_string())?;
        for s in pcm {
            writer.write_sample(*s).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_bytes_have_riff_header() {
        let bytes = pcm_to_wav_bytes(&[0, 100, -100], 16_000).unwrap();
        assert!(bytes.len() > 44);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
    }
}
