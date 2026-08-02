use crate::domain::i18n::Locale;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SttProvider {
    #[default]
    Local,
    /// Frontend and docs use `openrouter` (no underscore).
    #[serde(rename = "openrouter", alias = "open_router")]
    OpenRouter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    #[default]
    Local,
    #[serde(rename = "openrouter", alias = "open_router")]
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
    /// Transcription language hint (`auto`, `en`, `pt`, …)
    pub language: String,
    /// UI locale: `en` | `pt-BR`
    #[serde(default = "default_ui_locale")]
    pub ui_locale: String,
    #[serde(default)]
    pub onboarding_complete: bool,
    /// Selected microphone device id (flexaudio stable id)
    #[serde(default)]
    pub mic_device_id: Option<String>,
    /// Selected system/loopback device id
    #[serde(default)]
    pub system_device_id: Option<String>,
    /// Preferred compute backend: `cpu` | `cuda` | `auto`
    #[serde(default = "default_backend")]
    pub compute_backend: String,
    /// Whether to show the reminder to tell the room they are being recorded.
    /// Defaults on: a meeting tool that records other people should say so at
    /// least once, and the user can turn it off after the first time.
    #[serde(default = "default_true")]
    pub confirm_before_recording: bool,
    /// OpenRouter chat models the user picked, most recent first, capped at five.
    ///
    /// `#[serde(default)]` is load-bearing: `AppState::new` reads the row with
    /// `unwrap_or_default()`, so a field this struct requires and an older row
    /// does not carry would reset every existing user's settings in silence.
    #[serde(default)]
    pub recent_openrouter_llm_models: Vec<String>,
    /// UI theme: `light` | `dark`. Light is the product default.
    ///
    /// `#[serde(default)]` is load-bearing for the reason the field above already
    /// states: an older row does not carry this field, and `unwrap_or_default()`
    /// would turn the failure into a factory-fresh configuration in silence.
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_ui_locale() -> String {
    "en".into()
}
fn default_theme() -> String {
    "light".into()
}
fn default_true() -> bool {
    true
}
fn default_backend() -> String {
    "auto".into()
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
            ui_locale: "en".into(),
            onboarding_complete: false,
            mic_device_id: None,
            system_device_id: None,
            compute_backend: "auto".into(),
            confirm_before_recording: true,
            recent_openrouter_llm_models: Vec::new(),
            theme: "light".into(),
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
    pub fn locale(&self) -> Locale {
        Locale::from_code(&self.ui_locale)
    }

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

    /// Records a model pick, newest first, keeping at most five.
    ///
    /// Removing before inserting is what makes re-picking an old model promote it
    /// instead of leaving a duplicate behind in the list.
    pub fn remember_recent_llm_model(&mut self, id: &str) {
        self.recent_openrouter_llm_models.retain(|m| m != id);
        self.recent_openrouter_llm_models.insert(0, id.to_string());
        self.recent_openrouter_llm_models.truncate(5);
    }

    pub fn public_view(&self) -> AppSettings {
        let mut v = self.clone();
        if let Some(k) = &self.openrouter_api_key {
            // Slice by chars, not bytes: `&k[..4]` panics when byte 4 lands inside a
            // multi-byte char, and this runs on the boot path via `get_settings`.
            if k.chars().count() > 8 {
                let head: String = k.chars().take(4).collect();
                v.openrouter_api_key = Some(format!("{head}…"));
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
        assert!(!s.onboarding_complete);
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
        let s = AppSettings {
            openrouter_api_key: Some("sk-abcdefghij".into()),
            ..Default::default()
        };
        let p = s.public_view();
        assert_ne!(p.openrouter_api_key.as_deref(), Some("sk-abcdefghij"));
    }

    #[test]
    fn public_view_survives_multibyte_key() {
        // Byte 4 falls inside a char here — slicing by byte would panic on the
        // boot path, leaving the app stuck on the loading screen.
        let s = AppSettings {
            openrouter_api_key: Some("sk-ção-chave-secreta".into()),
            ..Default::default()
        };
        let p = s.public_view();
        let redacted = p.openrouter_api_key.unwrap();
        assert!(redacted.ends_with('…'));
        assert!(!redacted.contains("secreta"));
    }

    #[test]
    fn device_ids_roundtrip_serde() {
        let s = AppSettings {
            mic_device_id: Some("mic-1".into()),
            system_device_id: Some("sys-1".into()),
            onboarding_complete: true,
            ui_locale: "pt-BR".into(),
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        let s2: AppSettings = serde_json::from_str(&j).unwrap();
        assert_eq!(s2.mic_device_id.as_deref(), Some("mic-1"));
        assert_eq!(s2.system_device_id.as_deref(), Some("sys-1"));
        assert!(s2.onboarding_complete);
        assert_eq!(s2.locale(), Locale::PtBr);
    }

    #[test]
    fn recent_models_default_when_absent_from_an_older_row() {
        // Exactly what a build before this field wrote. `AppState::new` loads with
        // `unwrap_or_default()`, so a deserialise failure here does not surface as
        // an error — it silently hands the user a factory-fresh configuration.
        let older_row = r#"{
            "stt_provider": "local",
            "llm_provider": "openrouter",
            "openrouter_api_key": null,
            "openrouter_stt_model": "openai/gpt-4o-mini-transcribe",
            "openrouter_llm_model": "anthropic/claude-sonnet-4",
            "local_stt_model": "whisper-small",
            "local_llm_model": "qwen2.5-1.5b",
            "reasoning_enabled": true,
            "auto_summarize": false,
            "language": "pt",
            "ui_locale": "pt-BR",
            "onboarding_complete": true,
            "mic_device_id": "mic-1",
            "system_device_id": "sys-1",
            "compute_backend": "cuda",
            "confirm_before_recording": false
        }"#;
        let s: AppSettings = serde_json::from_str(older_row).expect("older rows must still load");
        assert_eq!(s.llm_provider, LlmProvider::OpenRouter);
        assert_eq!(s.openrouter_llm_model, "anthropic/claude-sonnet-4");
        assert_eq!(s.local_stt_model, "whisper-small");
        assert_eq!(s.ui_locale, "pt-BR");
        assert_eq!(s.mic_device_id.as_deref(), Some("mic-1"));
        assert_eq!(s.compute_backend, "cuda");
        assert!(!s.confirm_before_recording);
        assert!(s.recent_openrouter_llm_models.is_empty());
        assert_eq!(s.theme, "light");
    }

    #[test]
    fn saving_a_new_model_moves_it_to_the_front_of_recents() {
        let mut s = AppSettings::default();
        for id in ["a", "b", "c", "d", "e", "f"] {
            s.remember_recent_llm_model(id);
        }
        assert_eq!(s.recent_openrouter_llm_models, ["f", "e", "d", "c", "b"]);
        s.remember_recent_llm_model("c");
        assert_eq!(s.recent_openrouter_llm_models, ["c", "f", "e", "d", "b"]);
    }
}
