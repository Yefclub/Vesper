//! Local light LLM path. When GGUF weights exist under models/, invoke sidecar-style
//! generation; otherwise use extractive/offline responders so the app works offline.

use crate::domain::chat::{offline_answer, ChatMessage};
use crate::domain::summary::{extractive_summary, MeetingInsights, SummaryTemplate};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct LocalLlm {
    models_dir: PathBuf,
}

impl LocalLlm {
    pub fn new() -> Self {
        Self {
            models_dir: crate::paths::models_dir(),
        }
    }

    pub fn with_models_dir(dir: PathBuf) -> Self {
        Self { models_dir: dir }
    }

    pub fn model_file(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id).join("model.gguf")
    }

    pub fn is_ready(&self, model_id: &str) -> bool {
        self.model_file(model_id).is_file()
    }

    pub fn summarize(
        &self,
        transcript: &str,
        template: SummaryTemplate,
        model_id: &str,
    ) -> Result<MeetingInsights, String> {
        if self.is_ready(model_id) {
            // Weights present: still use extractive+template until llama.cpp sidecar is linked;
            // mark source so callers know local weights were detected.
            let mut insights = extractive_summary(transcript, 5);
            insights.summary = format!("({}) {}", template.id(), insights.summary);
            return Ok(insights);
        }
        Ok(extractive_summary(transcript, 4))
    }

    pub fn chat(
        &self,
        messages: &[ChatMessage],
        transcript: &str,
        _model_id: &str,
    ) -> Result<String, String> {
        let question = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        Ok(offline_answer(transcript, question))
    }
}

impl Default for LocalLlm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn summarize_offline() {
        let llm = LocalLlm::with_models_dir(tempdir().unwrap().path().to_path_buf());
        let i = llm
            .summarize(
                "Me: we need to ship auth. Others: agreed. TODO write tests.",
                SummaryTemplate::General,
                "qwen2.5-1.5b",
            )
            .unwrap();
        assert!(!i.summary.is_empty());
    }
}
