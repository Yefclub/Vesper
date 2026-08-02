//! OpenRouter model catalog.
//! STT: GET /api/v1/models?output_modalities=transcription
//! LLM: GET /api/v1/models (text chat models; skip audio/embed/image)

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrModel {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// What the model charges, per million tokens, as the picker shows it.
    ///
    /// `None` when the catalogue did not quote a price — the built-in fallback
    /// list has none, and a missing price is unknown rather than free.
    #[serde(default)]
    pub price_label: Option<String>,
}

/// The `pricing` block OpenRouter attaches to every catalogue entry.
///
/// Quoted per token as decimal strings, which is unreadable at that scale — the
/// formatting into "per million" lives in `domain::cost` with its tests.
fn price_of(m: &Value) -> Option<String> {
    let p = m.get("pricing")?;
    crate::domain::cost::format_price_per_mtok(
        p.get("prompt").and_then(|v| v.as_str()),
        p.get("completion").and_then(|v| v.as_str()),
    )
}

/// Parse OpenRouter models JSON into STT/transcription models.
/// Prefer `output_modalities=transcription` responses; also accepts full catalog
/// and filters by id/architecture/output_modalities.
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

        if is_stt_model(m, &id) {
            let price_label = price_of(m);
            out.push(OrModel {
                id,
                name,
                kind: "stt".into(),
                price_label,
            });
        }
    }

    if out.is_empty() {
        out.extend(default_stt_models());
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

fn is_stt_model(m: &Value, id: &str) -> bool {
    let id_l = id.to_ascii_lowercase();
    if id_l.contains("transcri")
        || id_l.contains("whisper")
        || (id_l.contains("voxtral") && id_l.contains("transcri"))
        || id_l.contains("speech-to-text")
        || id_l.contains("stt")
    {
        return true;
    }
    // output_modalities: ["transcription"] from filtered API.
    //
    // Only "transcription" counts. A model whose *output* is audio is a
    // text-to-speech model, and accepting it here put voices in the list of
    // things offered to transcribe a meeting.
    if let Some(arr) = m.get("output_modalities").and_then(|x| x.as_array()) {
        if arr.iter().any(|v| {
            v.as_str()
                .map(|s| s.eq_ignore_ascii_case("transcription"))
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    if let Some(arr) = m
        .pointer("/architecture/output_modalities")
        .and_then(|x| x.as_array())
    {
        if arr.iter().any(|v| {
            v.as_str()
                .map(|s| s.eq_ignore_ascii_case("transcription"))
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    let modality = m
        .pointer("/architecture/modality")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if modality.contains("audio->text") || modality.contains("audio→text") {
        return true;
    }
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
    // audio input + text output, but not TTS models
    if id_l.contains("tts") || id_l.ends_with("-tts") {
        return false;
    }
    input_mod.contains("audio")
        && (modality.contains("text") || modality.contains("audio->text") || modality.is_empty())
}

pub fn default_stt_models() -> Vec<OrModel> {
    vec![
        OrModel {
            id: "openai/gpt-4o-mini-transcribe".into(),
            name: "OpenAI: GPT-4o Mini Transcribe".into(),
            kind: "stt".into(),
            price_label: None,
        },
        OrModel {
            id: "openai/gpt-4o-transcribe".into(),
            name: "OpenAI: GPT-4o Transcribe".into(),
            kind: "stt".into(),
            price_label: None,
        },
        OrModel {
            id: "openai/whisper-1".into(),
            name: "OpenAI: Whisper".into(),
            kind: "stt".into(),
            price_label: None,
        },
        OrModel {
            id: "mistralai/voxtral-mini-2507".into(),
            name: "Mistral: Voxtral Mini Transcribe".into(),
            kind: "stt".into(),
            price_label: None,
        },
    ]
}

pub fn default_llm_models() -> Vec<OrModel> {
    vec![
        OrModel {
            id: "openai/gpt-4o-mini".into(),
            name: "OpenAI: GPT-4o Mini".into(),
            kind: "llm".into(),
            price_label: None,
        },
        OrModel {
            id: "openai/gpt-4o".into(),
            name: "OpenAI: GPT-4o".into(),
            kind: "llm".into(),
            price_label: None,
        },
        OrModel {
            id: "anthropic/claude-sonnet-4".into(),
            name: "Anthropic: Claude Sonnet 4".into(),
            kind: "llm".into(),
            price_label: None,
        },
        OrModel {
            id: "google/gemini-2.5-flash".into(),
            name: "Google: Gemini 2.5 Flash".into(),
            kind: "llm".into(),
            price_label: None,
        },
        OrModel {
            id: "meta-llama/llama-3.3-70b-instruct".into(),
            name: "Meta: Llama 3.3 70B Instruct".into(),
            kind: "llm".into(),
            price_label: None,
        },
        OrModel {
            id: "qwen/qwen-2.5-72b-instruct".into(),
            name: "Qwen: 2.5 72B Instruct".into(),
            kind: "llm".into(),
            price_label: None,
        },
    ]
}

/// Parse text/chat LLM models from OpenRouter catalog.
pub fn parse_openrouter_llm_models(body: &str) -> Result<Vec<OrModel>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "missing data array".to_string())?;
    // `links.next` is ignored: the whole catalog arrives in one body today. If it
    // ever comes back non-null this reads page one and silently drops the rest.
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
        if !is_text_llm(m, &id) {
            continue;
        }
        let name = m
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(&id)
            .to_string();
        let price_label = price_of(m);
        out.push(OrModel {
            id,
            name,
            kind: "llm".into(),
            price_label,
        });
    }
    if out.is_empty() {
        out.extend(default_llm_models());
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

fn is_text_llm(m: &Value, id: &str) -> bool {
    // Explicit output modalities, nested under `architecture` — the shape the API
    // actually sends. Reading the top-level field instead made this branch dead
    // against production and let every audio model through the catch-all below.
    if let Some(arr) = m
        .pointer("/architecture/output_modalities")
        .and_then(|x| x.as_array())
    {
        let outs: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        if !outs.is_empty() {
            return outs.contains(&"text") && !outs.contains(&"image") && !outs.contains(&"audio");
        }
    }
    // Fallback for entries carrying no modalities at all — i.e. the built-in
    // `default_llm_models()`. Guessing from the id is why `image` is gone from
    // this list: the modality check above carries that weight now, and substring
    // matching would eventually reject a chat model named `…-imagen-reasoning`.
    let id_l = id.to_ascii_lowercase();
    if id_l.contains("embed")
        || id_l.contains("whisper")
        || id_l.contains("transcri")
        || id_l.contains("tts")
        || id_l.contains("moderation")
        || id_l.contains("rerank")
        || id_l.contains("vision-preview") && id_l.contains("only")
    {
        return false;
    }
    let modality = m
        .pointer("/architecture/modality")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if modality.contains("text") {
        return true;
    }
    // Default catalog items without architecture: assume chat LLM
    true
}

async fn openrouter_get(api_key: &str, url: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let res = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("HTTP-Referer", "https://github.com/Yefclub/Vesper")
        .header("X-Title", "Vesper")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("OpenRouter models HTTP {}", res.status()));
    }
    res.text().await.map_err(|e| e.to_string())
}

/// Live STT catalog: official transcription modality filter.
pub async fn fetch_openrouter_stt_models(api_key: &str) -> Result<Vec<OrModel>, String> {
    if api_key.trim().is_empty() || api_key.contains('…') {
        return Ok(default_stt_models());
    }
    // Docs (2026): STT models are NOT in the default catalog — filter required.
    match openrouter_get(
        api_key,
        "https://openrouter.ai/api/v1/models?output_modalities=transcription",
    )
    .await
    {
        Ok(body) => parse_openrouter_stt_models(&body),
        Err(_) => Ok(default_stt_models()),
    }
}

/// Live text/LLM catalog.
pub async fn fetch_openrouter_llm_models(api_key: &str) -> Result<Vec<OrModel>, String> {
    if api_key.trim().is_empty() || api_key.contains('…') {
        return Ok(default_llm_models());
    }
    match openrouter_get(api_key, "https://openrouter.ai/api/v1/models").await {
        Ok(body) => parse_openrouter_llm_models(&body),
        Err(_) => Ok(default_llm_models()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STT_FIXTURE: &str = r#"{
      "data": [
        {"id": "openai/gpt-4o-mini-transcribe", "name": "GPT-4o Mini Transcribe", "output_modalities": ["transcription"]},
        {"id": "openai/whisper-1", "name": "Whisper", "output_modalities": ["transcription"]},
        {"id": "openai/gpt-4o-mini", "name": "GPT-4o Mini", "output_modalities": ["text"]}
      ]
    }"#;

    // `output_modalities` is nested under `architecture` here because that is the
    // only place OpenRouter puts it. The earlier fixture invented a top-level
    // field, so this test passed against a payload the API never sends.
    const LLM_FIXTURE: &str = r#"{
      "data": [
        {"id": "openai/gpt-4o-mini", "name": "GPT-4o Mini", "architecture": {"modality": "text->text", "output_modalities": ["text"]}},
        {"id": "openai/gpt-4o-mini-transcribe", "name": "Mini Transcribe", "architecture": {"output_modalities": ["transcription"]}},
        {"id": "openai/whisper-1", "name": "Whisper", "architecture": {"output_modalities": ["transcription"]}},
        {"id": "anthropic/claude-sonnet-4", "name": "Claude Sonnet 4", "architecture": {"output_modalities": ["text"]}},
        {"id": "openai/text-embedding-3-small", "name": "Embed", "architecture": {"output_modalities": ["embeddings"]}}
      ]
    }"#;

    #[test]
    fn stt_from_transcription_modality() {
        let models = parse_openrouter_stt_models(STT_FIXTURE).unwrap();
        assert!(models.iter().any(|m| m.id.contains("transcribe")));
        assert!(models.iter().any(|m| m.id.contains("whisper")));
        assert!(!models.iter().any(|m| m.id == "openai/gpt-4o-mini"));
    }

    #[test]
    fn stt_empty_falls_back() {
        let body =
            r#"{"data":[{"id":"openai/gpt-4o-mini","name":"Mini","output_modalities":["text"]}]}"#;
        let models = parse_openrouter_stt_models(body).unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.kind == "stt"));
    }

    #[test]
    fn llm_lists_text_skips_stt_and_embed() {
        let models = parse_openrouter_llm_models(LLM_FIXTURE).unwrap();
        assert!(models.iter().any(|m| m.id.contains("claude")));
        assert!(models.iter().any(|m| m.id == "openai/gpt-4o-mini"));
        assert!(!models.iter().any(|m| m.id.contains("whisper")));
        assert!(!models.iter().any(|m| m.id.contains("transcribe")));
        assert!(!models.iter().any(|m| m.id.contains("embed")));
    }

    #[test]
    fn a_two_hundred_entry_catalogue_comes_back_whole() {
        // The list used to be sorted by display name and cut at 120. That is not a
        // "top slice", it is a prefix of the alphabet: the live catalog lost every
        // OpenAI and Qwen model, including the app's own default.
        let entries: Vec<String> = (0..200)
            .map(|i| {
                format!(
                    r#"{{"id":"vendor/model-{i:03}","name":"Vendor: Model {i:03}","architecture":{{"output_modalities":["text"]}}}}"#
                )
            })
            .collect();
        let body = format!(r#"{{"data":[{}]}}"#, entries.join(","));
        let models = parse_openrouter_llm_models(&body).unwrap();
        assert_eq!(models.len(), 200);
        assert!(models.iter().any(|m| m.id == "vendor/model-199"));
    }

    #[test]
    fn nested_architecture_output_modalities_is_what_gets_read() {
        // Two places to look, and only the nested one is what OpenRouter sends —
        // so the nested one has to be the one that decides.
        let body = r#"{"data":[
            {"id":"vendor/really-an-image-model","name":"Image","output_modalities":["text"],
             "architecture":{"output_modalities":["image","text"]}},
            {"id":"vendor/really-a-chat-model","name":"Chat","output_modalities":["image"],
             "architecture":{"output_modalities":["text"]}}
        ]}"#;
        let models = parse_openrouter_llm_models(body).unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["vendor/really-a-chat-model"]);
    }

    #[test]
    fn gpt_4o_mini_and_qwen_survive_the_filter() {
        // Shapes copied from the live catalog. Both were cut by the truncate, and
        // `openai/gpt-4o-mini` is the model the app ships as its default — note it
        // takes image *input*, which must not be read as image output.
        let body = r#"{"data":[
            {"id":"openai/gpt-4o-mini","name":"OpenAI: GPT-4o-mini",
             "architecture":{"modality":"text+image+file->text","input_modalities":["text","image","file"],"output_modalities":["text"]}},
            {"id":"qwen/qwen-2.5-72b-instruct","name":"Qwen: 2.5 72B Instruct",
             "architecture":{"modality":"text->text","input_modalities":["text"],"output_modalities":["text"]}}
        ]}"#;
        let models = parse_openrouter_llm_models(body).unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"openai/gpt-4o-mini"), "{ids:?}");
        assert!(ids.contains(&"qwen/qwen-2.5-72b-instruct"), "{ids:?}");
    }

    #[test]
    fn serde_provider_accepts_openrouter_string() {
        use crate::domain::settings::{LlmProvider, SttProvider};
        let s: SttProvider = serde_json::from_str("\"openrouter\"").unwrap();
        assert_eq!(s, SttProvider::OpenRouter);
        let l: LlmProvider = serde_json::from_str("\"openrouter\"").unwrap();
        assert_eq!(l, LlmProvider::OpenRouter);
        // also accept snake_case alias
        let s2: SttProvider = serde_json::from_str("\"open_router\"").unwrap();
        assert_eq!(s2, SttProvider::OpenRouter);
    }
}

#[cfg(test)]
mod audio_modality_tests {
    use super::*;

    #[test]
    fn a_model_that_outputs_audio_is_not_a_transcriber() {
        // Text-to-speech: it emits audio, it does not read it. Offering one as an
        // STT choice means the user picks a voice to transcribe their meeting.
        let body = r#"{"data":[
            {"id":"vendor/some-tts","name":"Some TTS","output_modalities":["audio"]},
            {"id":"vendor/real-stt","name":"Real STT","output_modalities":["transcription"]}
        ]}"#;
        let models = parse_openrouter_stt_models(body).unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"vendor/real-stt"), "{ids:?}");
        assert!(!ids.contains(&"vendor/some-tts"), "{ids:?}");
    }

    #[test]
    fn an_audio_output_model_is_not_a_chat_model() {
        // `openai/gpt-audio` outputs ["text","audio"]. It passed the old filter as
        // a chat model, so a voice model was on offer for summarising a meeting.
        let body = r#"{"data":[
            {"id":"openai/gpt-audio","name":"OpenAI: GPT Audio","architecture":{"output_modalities":["text","audio"]}},
            {"id":"openai/gpt-4o-mini","name":"OpenAI: GPT-4o-mini","architecture":{"output_modalities":["text"]}}
        ]}"#;
        let models = parse_openrouter_llm_models(body).unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"openai/gpt-4o-mini"), "{ids:?}");
        assert!(!ids.contains(&"openai/gpt-audio"), "{ids:?}");
    }
}
