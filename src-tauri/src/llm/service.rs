use crate::domain::chat::{build_chat_context, ChatMessage};
use crate::domain::settings::{AppSettings, LlmProvider};
use crate::domain::summary::{MeetingInsights, SummaryTemplate};
use crate::llm::local::LocalLlm;
use crate::llm::openrouter::OpenRouterLlm;

pub struct LlmService {
    local: LocalLlm,
    remote: OpenRouterLlm,
}

impl LlmService {
    pub fn new() -> Self {
        Self {
            local: LocalLlm::new(),
            remote: OpenRouterLlm::new(),
        }
    }

    pub async fn summarize(
        &self,
        settings: &AppSettings,
        transcript: &str,
        template: SummaryTemplate,
    ) -> Result<MeetingInsights, String> {
        match settings.llm_provider {
            LlmProvider::Local => {
                self.local
                    .summarize(transcript, template, &settings.local_llm_model)
            }
            LlmProvider::OpenRouter => {
                settings
                    .require_openrouter_key()
                    .map_err(|e| e.to_string())?;
                self.remote
                    .summarize(
                        settings.openrouter_api_key.as_deref().unwrap_or(""),
                        &settings.openrouter_llm_model,
                        transcript,
                        template,
                        settings.reasoning_enabled,
                    )
                    .await
            }
        }
    }

    pub async fn chat(
        &self,
        settings: &AppSettings,
        meeting_title: &str,
        transcript: &str,
        summary: Option<&str>,
        history: &[ChatMessage],
        question: &str,
    ) -> Result<String, String> {
        let messages = build_chat_context(
            meeting_title,
            transcript,
            summary,
            history,
            question,
            12_000,
        );
        match settings.llm_provider {
            LlmProvider::Local => self
                .local
                .chat(&messages, transcript, &settings.local_llm_model),
            LlmProvider::OpenRouter => {
                settings
                    .require_openrouter_key()
                    .map_err(|e| e.to_string())?;
                self.remote
                    .complete(
                        settings.openrouter_api_key.as_deref().unwrap_or(""),
                        &settings.openrouter_llm_model,
                        &messages,
                        settings.reasoning_enabled,
                    )
                    .await
            }
        }
    }
}

impl Default for LlmService {
    fn default() -> Self {
        Self::new()
    }
}
