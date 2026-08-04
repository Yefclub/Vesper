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

    /// The client, key and model for whichever remote provider is selected.
    ///
    /// Both speak OpenAI's chat-completions protocol, so every call site wants
    /// the same three things and differs only in where they came from. The
    /// address is validated here rather than when it was typed: a settings file
    /// written by an older build, or edited by hand, must not be able to send a
    /// transcript somewhere the rule would have refused.
    fn remote_for(
        &self,
        settings: &AppSettings,
    ) -> Result<(OpenRouterLlm, String, String), String> {
        match settings.llm_provider {
            LlmProvider::OpenAiCompatible => {
                let base =
                    crate::domain::endpoint::validate_endpoint_url(&settings.endpoint_base_url)
                        .map_err(|e| e.to_string())?;
                if settings.endpoint_model.trim().is_empty() {
                    return Err("choose a model on the endpoint".into());
                }
                Ok((
                    OpenRouterLlm::at(base),
                    String::new(),
                    settings.endpoint_model.clone(),
                ))
            }
            _ => {
                settings
                    .require_openrouter_key()
                    .map_err(|e| e.to_string())?;
                Ok((
                    self.remote.clone(),
                    settings.openrouter_api_key.clone().unwrap_or_default(),
                    settings.openrouter_llm_model.clone(),
                ))
            }
        }
    }

    pub async fn summarize(
        &self,
        settings: &AppSettings,
        transcript: &str,
        template: SummaryTemplate,
        notes: &[crate::domain::context::ContextNote],
    ) -> Result<(MeetingInsights, Option<i64>), String> {
        match settings.llm_provider {
            // A local model costs nothing, which is not the same as costing
            // zero: `None` is what stops a meeting that never left the machine
            // from displaying a price at all.
            LlmProvider::Local => {
                let local = self.local.clone();
                let transcript = transcript.to_string();
                let locale = settings.locale();
                let model = settings.local_llm_model.clone();
                let backend = settings.compute_backend.clone();
                let reasoning = settings.reasoning_enabled;
                let notes = notes.to_vec();
                tokio::task::spawn_blocking(move || {
                    local.summarize(
                        crate::domain::context::SummarySubject {
                            transcript: &transcript,
                            template,
                            locale,
                            notes: &notes,
                        },
                        &model,
                        &backend,
                        reasoning,
                    )
                })
                .await
                .map_err(|e| e.to_string())?
                .map(|insights| (insights, None))
            }
            LlmProvider::OpenRouter | LlmProvider::OpenAiCompatible => {
                let (client, key, model) = self.remote_for(settings)?;
                client
                    .summarize(
                        &key,
                        &model,
                        crate::domain::context::SummarySubject {
                            transcript,
                            template,
                            locale: settings.locale(),
                            notes,
                        },
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
            // window what it is doing.
            LlmProvider::Local => {
                let local = self.local.clone();
                let model = settings.local_llm_model.clone();
                let backend = settings.compute_backend.clone();
                tokio::task::spawn_blocking(move || local.title(&prompt, &model, &backend))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|title| (title, None))
            }
            LlmProvider::OpenRouter | LlmProvider::OpenAiCompatible => {
                let (client, key, model) = self.remote_for(settings)?;
                let messages = vec![ChatMessage {
                    role: "user".into(),
                    content: prompt,
                }];
                client
                    .complete(
                        &key, &model, &messages,
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
                let backend = settings.compute_backend.clone();
                tokio::task::spawn_blocking(move || local.chat(&messages, "", &model, &backend))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|answer| (answer, None))
            }
            LlmProvider::OpenRouter | LlmProvider::OpenAiCompatible => {
                let (client, key, model) = self.remote_for(settings)?;
                client.complete(&key, &model, messages, false).await
            }
        }
    }

    pub async fn chat(
        &self,
        settings: &AppSettings,
        subject: crate::domain::context::ChatSubject<'_>,
        history: &[ChatMessage],
        question: &str,
    ) -> Result<(String, Option<i64>), String> {
        let messages = build_chat_context(
            subject.meeting_title,
            subject.transcript,
            subject.summary,
            history,
            question,
            12_000,
            subject.notes,
        );
        match settings.llm_provider {
            LlmProvider::Local => {
                let local = self.local.clone();
                let transcript = subject.transcript.to_string();
                let model = settings.local_llm_model.clone();
                let backend = settings.compute_backend.clone();
                tokio::task::spawn_blocking(move || {
                    local.chat(&messages, &transcript, &model, &backend)
                })
                .await
                .map_err(|e| e.to_string())?
                .map(|answer| (answer, None))
            }
            LlmProvider::OpenRouter | LlmProvider::OpenAiCompatible => {
                let (client, key, model) = self.remote_for(settings)?;
                client
                    .complete(&key, &model, &messages, settings.reasoning_enabled)
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
