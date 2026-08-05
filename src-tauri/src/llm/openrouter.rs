use crate::domain::chat::ChatMessage;
use crate::domain::cost::usd_to_nano;
use crate::domain::summary::{build_summary_prompt_with, MeetingInsights};
use serde_json::json;

/// A chat-completions client, pointed at OpenRouter by default.
///
/// The protocol is OpenAI's, which is also what Ollama, LM Studio and vLLM
/// speak, so one implementation serves both. What differs is small enough to
/// name: OpenRouter is asked to price the call and gets two headers that
/// identify this application, and a server on the user's own machine gets
/// neither — there is nobody there to bill and nobody there to identify to.
#[derive(Debug, Clone)]
pub struct OpenRouterLlm {
    base_url: String,
    /// Whether this is OpenRouter rather than a server the user runs.
    branded: bool,
}

impl OpenRouterLlm {
    pub fn new() -> Self {
        Self {
            base_url: "https://openrouter.ai/api/v1".into(),
            branded: true,
        }
    }

    /// The same protocol at an address the user chose. Validated by
    /// `domain::endpoint` before it reaches here — this type takes the string
    /// it is given.
    pub fn at(base_url: String) -> Self {
        Self {
            base_url,
            branded: false,
        }
    }

    pub async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        reasoning: bool,
    ) -> Result<(String, Option<i64>), String> {
        // A local server usually wants no key at all, and refusing to call it
        // without one would make the common case impossible.
        if self.branded && api_key.trim().is_empty() {
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
        if self.branded {
            // Without this OpenRouter answers with token counts and no price.
            // The whole point of reading `usage.cost` is that the provider has
            // already applied whatever discount or free tier this key gets, so
            // deriving it from tokens here would be a different number.
            body["usage"] = json!({ "include": true });
        }
        if reasoning {
            body["include_reasoning"] = json!(true);
            // Some OpenRouter reasoning models accept this flag family.
            body["reasoning"] = json!({ "effort": "medium" });
        }

        // A redirect out of an approved address undoes the approval. reqwest
        // follows 307 and 308 with the POST body intact, so a server on
        // localhost could answer "moved" and have the transcript delivered to
        // whatever host it named — an address that never passed the check.
        // OpenRouter keeps the default policy; it is a public API that has
        // always been allowed to move its own endpoints.
        let client = if self.branded {
            reqwest::Client::new()
        } else {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                // No proxy, ever, for an address the user configured. The
                // allowlist checks the host in the URL, but a proxy makes the
                // peer somebody else entirely — `HTTP_PROXY` set in the
                // environment would carry a transcript bound for 127.0.0.1
                // straight out to a public host, with the check having passed.
                .no_proxy()
                .build()
                .map_err(|e| e.to_string())?
        };
        let mut req = client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if !api_key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {api_key}"));
        }
        if self.branded {
            req = req
                .header("HTTP-Referer", "https://github.com/Yefclub/Vesper")
                .header("X-Title", "Vesper");
        }
        let res = req.send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            let status = res.status();
            let t = res.text().await.unwrap_or_default();
            let who = if self.branded {
                "OpenRouter LLM"
            } else {
                "the LLM endpoint"
            };
            return Err(format!("{who} {status}: {t}"));
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
        // A server the user runs bills nobody, and reporting zero would put a
        // price on a meeting that never had one.
        let cost = self
            .branded
            .then(|| v["usage"]["cost"].as_f64().and_then(usd_to_nano))
            .flatten();
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
        Ok((MeetingInsights::from_model_text_for(template, &raw), cost))
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
