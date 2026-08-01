//! OpenRouter audio transcription client (`/api/v1/audio/transcriptions`).

use hound::{WavSpec, WavWriter};
use std::io::Cursor;

#[derive(Debug, Clone, Default)]
pub struct OpenRouterStt {
    base_url: String,
}

impl OpenRouterStt {
    pub fn new() -> Self {
        Self {
            base_url: "https://openrouter.ai/api/v1".into(),
        }
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    pub async fn transcribe(
        &self,
        pcm: &[i16],
        sample_rate: u32,
        api_key: &str,
        model: &str,
        language: &str,
    ) -> Result<String, String> {
        if api_key.trim().is_empty() {
            return Err("OpenRouter API key required".into());
        }
        let wav = pcm_to_wav_bytes(pcm, sample_rate)?;
        let client = reqwest::Client::new();
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

        let res = client
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
        Ok(v.get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string())
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
        let mut writer =
            WavWriter::new(&mut cursor, spec).map_err(|e| e.to_string())?;
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
