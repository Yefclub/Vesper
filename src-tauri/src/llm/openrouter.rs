use crate::domain::chat::ChatMessage;
use crate::domain::cost::usd_to_nano;
use crate::domain::summary::{build_summary_prompt_with, MeetingInsights};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct OpenRouterLlm {
    base_url: String,
}

impl OpenRouterLlm {
    pub fn new() -> Self {
        Self {
            base_url: "https://openrouter.ai/api/v1".into(),
        }
    }

    pub async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        reasoning: bool,
    ) -> Result<(String, Option<i64>), String> {
        if api_key.trim().is_empty() {
            return Err("OpenRouter API key required".into());
        }
        let msgs: Vec<_> = messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();
        let mut body = json!({
            "model": model,
            "messages": msgs,
            // Without this OpenRouter answers with token counts and no price.
            // The whole point of reading `usage.cost` is that the provider has
            // already applied whatever discount or free tier this key gets, so
            // deriving it from tokens here would be a different number.
            "usage": { "include": true },
        });
        if reasoning {
            body["include_reasoning"] = json!(true);
            // Some OpenRouter reasoning models accept this flag family.
            body["reasoning"] = json!({ "effort": "medium" });
        }

        let client = reqwest::Client::new();
        let res = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {api_key}"))
            .header("HTTP-Referer", "https://github.com/Yefclub/Vesper")
            .header("X-Title", "Vesper")
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            let status = res.status();
            let t = res.text().await.unwrap_or_default();
            return Err(format!("OpenRouter LLM {status}: {t}"));
        }
        let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        let content = v["choices"]
            .get(0)
            .and_then(|c| c["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();
        // Absent rather than zero when the field does not arrive: a call whose
        // price we never learned is unknown, and recording it as free would
        // quietly understate what the meeting cost.
        let cost = v["usage"]["cost"].as_f64().and_then(usd_to_nano);
        Ok((content, cost))
    }

    pub async fn summarize(
        &self,
        api_key: &str,
        model: &str,
        subject: crate::domain::context::SummarySubject<'_>,
        reasoning: bool,
    ) -> Result<(MeetingInsights, Option<i64>), String> {
        let crate::domain::context::SummarySubject {
            transcript,
            template,
            locale,
            notes,
        } = subject;
        let prompt = build_summary_prompt_with(template, transcript, locale, notes);
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: "You are a precise meeting notes assistant.".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: prompt,
            },
        ];
        let (raw, cost) = self.complete(api_key, model, &messages, reasoning).await?;
        Ok((MeetingInsights::from_model_text(&raw), cost))
    }
}

impl Default for OpenRouterLlm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_without_network() {
        let _ = OpenRouterLlm::new();
    }
}
