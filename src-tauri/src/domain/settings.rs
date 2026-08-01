use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SttProvider {
    #[default]
    Local,
    OpenRouter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    #[default]
    Local,
    OpenRouter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub stt_provider: SttProvider,
    pub llm_provider: LlmProvider,
    pub openrouter_api_key: Option<String>,
    pub openrouter_stt_model: String,
    pub openrouter_llm_model: String,
    pub local_stt_model: String,
    pub local_llm_model: String,
    pub reasoning_enabled: bool,
    pub auto_summarize: bool,
    pub language: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            stt_provider: SttProvider::Local,
            llm_provider: LlmProvider::Local,
            openrouter_api_key: None,
            openrouter_stt_model: "openai/gpt-4o-mini-transcribe".into(),
            openrouter_llm_model: "openai/gpt-4o-mini".into(),
            local_stt_model: "whisper-tiny".into(),
            local_llm_model: "qwen2.5-0.5b".into(),
            reasoning_enabled: false,
            auto_summarize: true,
            language: "auto".into(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("OpenRouter provider selected but API key is missing")]
    MissingOpenRouterKey,
    #[error("invalid model id: empty")]
    EmptyModel,
}

impl AppSettings {
    pub fn switch_stt(&mut self, provider: SttProvider) -> Result<(), SettingsError> {
        if provider == SttProvider::OpenRouter {
            self.require_openrouter_key()?;
        }
        self.stt_provider = provider;
        Ok(())
    }

    pub fn switch_llm(&mut self, provider: LlmProvider) -> Result<(), SettingsError> {
        if provider == LlmProvider::OpenRouter {
            self.require_openrouter_key()?;
        }
        self.llm_provider = provider;
        Ok(())
    }

    pub fn set_reasoning(&mut self, enabled: bool) {
        self.reasoning_enabled = enabled;
    }

    pub fn require_openrouter_key(&self) -> Result<(), SettingsError> {
        match &self.openrouter_api_key {
            Some(k) if !k.trim().is_empty() => Ok(()),
            _ => Err(SettingsError::MissingOpenRouterKey),
        }
    }

    pub fn validate_models(&self) -> Result<(), SettingsError> {
        if self.local_stt_model.trim().is_empty()
            || self.local_llm_model.trim().is_empty()
            || self.openrouter_stt_model.trim().is_empty()
            || self.openrouter_llm_model.trim().is_empty()
        {
            return Err(SettingsError::EmptyModel);
        }
        Ok(())
    }

    /// Redacted view for UI (never return full API key).
    pub fn public_view(&self) -> AppSettings {
        let mut v = self.clone();
        if let Some(k) = &self.openrouter_api_key {
            if k.len() > 8 {
                v.openrouter_api_key = Some(format!("{}…", &k[..4]));
            } else if !k.is_empty() {
                v.openrouter_api_key = Some("****".into());
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_local_offline() {
        let s = AppSettings::default();
        assert_eq!(s.stt_provider, SttProvider::Local);
        assert_eq!(s.llm_provider, LlmProvider::Local);
        assert!(!s.reasoning_enabled);
    }

    #[test]
    fn switch_stt_requires_key_for_openrouter() {
        let mut s = AppSettings::default();
        assert!(s.switch_stt(SttProvider::OpenRouter).is_err());
        s.openrouter_api_key = Some("sk-test".into());
        assert!(s.switch_stt(SttProvider::OpenRouter).is_ok());
        assert_eq!(s.stt_provider, SttProvider::OpenRouter);
        assert!(s.switch_stt(SttProvider::Local).is_ok());
    }

    #[test]
    fn reasoning_toggle() {
        let mut s = AppSettings::default();
        s.set_reasoning(true);
        assert!(s.reasoning_enabled);
    }

    #[test]
    fn public_view_redacts_key() {
        let mut s = AppSettings::default();
        s.openrouter_api_key = Some("sk-abcdefghij".into());
        let p = s.public_view();
        assert_ne!(p.openrouter_api_key.as_deref(), Some("sk-abcdefghij"));
    }
}
