use crate::domain::chat::ChatMessage;
use crate::domain::summary::{build_summary_prompt, MeetingInsights, SummaryTemplate};
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
    ) -> Result<String, String> {
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
        Ok(content)
    }

    pub async fn summarize(
        &self,
        api_key: &str,
        model: &str,
        transcript: &str,
        template: SummaryTemplate,
        reasoning: bool,
    ) -> Result<MeetingInsights, String> {
        let prompt = build_summary_prompt(template, transcript);
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
        let raw = self.complete(api_key, model, &messages, reasoning).await?;
        Ok(MeetingInsights::from_model_text(&raw))
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
