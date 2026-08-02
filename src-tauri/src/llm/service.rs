use crate::domain::chat::{build_chat_context, ChatMessage};
use crate::domain::settings::{AppSettings, LlmProvider};
use crate::domain::summary::{MeetingInsights, SummaryTemplate};
use crate::domain::title::build_title_prompt;
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
    ) -> Result<(MeetingInsights, Option<i64>), String> {
        match settings.llm_provider {
            // A local model costs nothing, which is not the same as costing
            // zero: `None` is what stops a meeting that never left the machine
            // from displaying a price at all.
            LlmProvider::Local => self
                .local
                .summarize(
                    transcript,
                    template,
                    settings.locale(),
                    &settings.local_llm_model,
                )
                .map(|insights| (insights, None)),
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
                        settings.locale(),
                        settings.reasoning_enabled,
                    )
                    .await
            }
        }
    }

    /// Name a meeting from its summary and its own words.
    ///
    /// Deliberately not routed through `chat`: with no GGUF on disk the local
    /// path answers from `offline_answer`, and a keyword-matched transcript
    /// line is worse than the date label it would replace. A missing model is
    /// an `Err` here, which the caller treats as "keep the label".
    pub async fn title(
        &self,
        settings: &AppSettings,
        summary: &str,
        transcript: &str,
    ) -> Result<(String, Option<i64>), String> {
        let prompt = build_title_prompt(summary, transcript);
        match settings.llm_provider {
            // Loading a GGUF and generating from it takes seconds of pure CPU. On
            // a runtime worker that stalls every other task on the executor —
            // including the event pump that is, at this exact moment, telling the
            // window what it is doing. `summarize` above has the same shape and
            // predates this; it is pointed at, not changed here.
            LlmProvider::Local => {
                let local = self.local.clone();
                let model = settings.local_llm_model.clone();
                tokio::task::spawn_blocking(move || local.title(&prompt, &model))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|title| (title, None))
            }
            LlmProvider::OpenRouter => {
                settings
                    .require_openrouter_key()
                    .map_err(|e| e.to_string())?;
                let messages = vec![ChatMessage {
                    role: "user".into(),
                    content: prompt,
                }];
                self.remote
                    .complete(
                        settings.openrouter_api_key.as_deref().unwrap_or(""),
                        &settings.openrouter_llm_model,
                        &messages,
                        // Never reasoning, whatever the setting says: six words
                        // do not need a thinking budget, and on OpenRouter that
                        // budget is billed.
                        false,
                    )
                    .await
            }
        }
    }

    /// One completion, for a refinement.
    ///
    /// Its own entry point rather than reusing `chat`: chat builds a context out
    /// of the meeting and its history, and a refinement already carries
    /// everything it needs in one message. Never reasoning — the answer is a
    /// bullet list, and on OpenRouter a thinking budget is billed.
    pub async fn complete_for_refine(
        &self,
        settings: &AppSettings,
        messages: &[ChatMessage],
    ) -> Result<(String, Option<i64>), String> {
        match settings.llm_provider {
            LlmProvider::Local => {
                let local = self.local.clone();
                let model = settings.local_llm_model.clone();
                let messages = messages.to_vec();
                // No offline fallback here. The extractive path cannot reason,
                // and it would replace good notes with keyword soup.
                tokio::task::spawn_blocking(move || local.chat(&messages, "", &model))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|answer| (answer, None))
            }
            LlmProvider::OpenRouter => {
                settings
                    .require_openrouter_key()
                    .map_err(|e| e.to_string())?;
                self.remote
                    .complete(
                        settings.openrouter_api_key.as_deref().unwrap_or(""),
                        &settings.openrouter_llm_model,
                        messages,
                        false,
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
    ) -> Result<(String, Option<i64>), String> {
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
                .chat(&messages, transcript, &settings.local_llm_model)
                .map(|answer| (answer, None)),
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
