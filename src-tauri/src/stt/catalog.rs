//! OpenRouter model catalog parsing (STT / audio-capable models).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrModel {
    pub id: String,
    pub name: String,
    pub kind: String,
}

/// Parse OpenRouter `/api/v1/models` JSON into selectable STT models.
/// Prefers models whose id/architecture hints at audio/transcription.
pub fn parse_openrouter_stt_models(body: &str) -> Result<Vec<OrModel>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "missing data array".to_string())?;

    let mut out = Vec::new();
    for m in data {
        let id = m
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = m
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(&id)
            .to_string();
        let modality = m
            .pointer("/architecture/modality")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let input_mod = m
            .pointer("/architecture/input_modalities")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();

        let id_l = id.to_ascii_lowercase();
        let is_stt = id_l.contains("transcri")
            || id_l.contains("whisper")
            || id_l.contains("voxtral")
            || id_l.contains("speech")
            || modality.contains("audio")
            || input_mod.contains("audio");

        if is_stt {
            out.push(OrModel {
                id,
                name,
                kind: "stt".into(),
            });
        }
    }

    // Stable fallbacks if API returns nothing audio-tagged
    if out.is_empty() {
        out.extend(default_stt_models());
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

pub fn default_stt_models() -> Vec<OrModel> {
    vec![
        OrModel {
            id: "openai/gpt-4o-mini-transcribe".into(),
            name: "GPT-4o Mini Transcribe".into(),
            kind: "stt".into(),
        },
        OrModel {
            id: "openai/gpt-4o-transcribe".into(),
            name: "GPT-4o Transcribe".into(),
            kind: "stt".into(),
        },
        OrModel {
            id: "openai/whisper-large-v3".into(),
            name: "Whisper Large V3".into(),
            kind: "stt".into(),
        },
        OrModel {
            id: "google/gemini-2.5-flash-preview-tts".into(),
            name: "Gemini Flash (audio-capable)".into(),
            kind: "stt".into(),
        },
    ]
}

pub fn parse_openrouter_llm_models(body: &str) -> Result<Vec<OrModel>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "missing data array".to_string())?;
    let mut out = Vec::new();
    for m in data {
        let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let id_l = id.to_ascii_lowercase();
        // Skip pure embedding / image / audio-only if obvious
        if id_l.contains("embed") || id_l.contains("whisper") || id_l.contains("transcri") {
            continue;
        }
        let name = m
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(&id)
            .to_string();
        out.push(OrModel {
            id,
            name,
            kind: "llm".into(),
        });
    }
    out.truncate(80);
    Ok(out)
}

/// Live fetch when API key present.
pub async fn fetch_openrouter_stt_models(api_key: &str) -> Result<Vec<OrModel>, String> {
    if api_key.trim().is_empty() {
        return Ok(default_stt_models());
    }
    let client = reqwest::Client::new();
    let res = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("HTTP-Referer", "https://github.com/Yefclub/Vesper")
        .header("X-Title", "Vesper")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        // Fall back to curated list rather than failing onboarding
        return Ok(default_stt_models());
    }
    let body = res.text().await.map_err(|e| e.to_string())?;
    parse_openrouter_stt_models(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "data": [
        {"id": "openai/gpt-4o-mini", "name": "GPT-4o Mini", "architecture": {"modality": "text->text"}},
        {"id": "openai/gpt-4o-mini-transcribe", "name": "Mini Transcribe", "architecture": {"modality": "audio->text", "input_modalities": ["audio"]}},
        {"id": "openai/whisper-large-v3", "name": "Whisper", "architecture": {"input_modalities": ["audio"]}},
        {"id": "anthropic/claude-sonnet-4", "name": "Claude", "architecture": {"modality": "text->text"}}
      ]
    }"#;

    #[test]
    fn parses_audio_tagged_models() {
        let models = parse_openrouter_stt_models(FIXTURE).unwrap();
        assert!(models.iter().any(|m| m.id.contains("transcribe")));
        assert!(models.iter().any(|m| m.id.contains("whisper")));
        assert!(!models.iter().any(|m| m.id.contains("claude")));
    }

    #[test]
    fn empty_audio_falls_back_to_defaults() {
        let body = r#"{"data":[{"id":"openai/gpt-4o-mini","name":"Mini"}]}"#;
        let models = parse_openrouter_stt_models(body).unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.kind == "stt"));
    }

    #[test]
    fn llm_parser_skips_whisper() {
        let models = parse_openrouter_llm_models(FIXTURE).unwrap();
        assert!(models.iter().any(|m| m.id.contains("claude") || m.id.contains("gpt-4o-mini")));
        assert!(!models.iter().any(|m| m.id.contains("whisper")));
    }
}
