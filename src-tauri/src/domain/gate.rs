//! Recording start gate — pure, unit-tested without hardware.

use crate::domain::settings::{AppSettings, SttProvider};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartGate {
    pub allowed: bool,
    pub reason: Option<String>,
    pub reason_key: Option<String>,
}

/// Decide whether recording may start given settings + readiness flags.
pub fn can_start_recording(
    settings: &AppSettings,
    local_stt_ready: bool,
    onboarding_complete: bool,
) -> StartGate {
    if !onboarding_complete || !settings.onboarding_complete {
        return StartGate {
            allowed: false,
            reason: Some("Finish onboarding before recording.".into()),
            reason_key: Some("gate.onboarding".into()),
        };
    }
    match settings.stt_provider {
        SttProvider::Local => {
            if local_stt_ready {
                StartGate {
                    allowed: true,
                    reason: None,
                    reason_key: None,
                }
            } else {
                StartGate {
                    allowed: false,
                    reason: Some(format!(
                        "Local STT model `{}` is not installed. Download it in Settings or switch to OpenRouter.",
                        settings.local_stt_model
                    )),
                    reason_key: Some("gate.local_stt".into()),
                }
            }
        }
        SttProvider::OpenRouter => {
            let key_ok = settings
                .openrouter_api_key
                .as_ref()
                .map(|k| !k.trim().is_empty() && !k.contains('…') && k != "****")
                .unwrap_or(false);
            let model_ok = !settings.openrouter_stt_model.trim().is_empty();
            if key_ok && model_ok {
                StartGate {
                    allowed: true,
                    reason: None,
                    reason_key: None,
                }
            } else if !key_ok {
                StartGate {
                    allowed: false,
                    reason: Some("OpenRouter API key is required for cloud STT.".into()),
                    reason_key: Some("gate.openrouter_key".into()),
                }
            } else {
                StartGate {
                    allowed: false,
                    reason: Some("Select an OpenRouter STT model.".into()),
                    reason_key: Some("gate.openrouter_stt_model".into()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::{AppSettings, SttProvider};

    fn base() -> AppSettings {
        let mut s = AppSettings::default();
        s.onboarding_complete = true;
        s
    }

    #[test]
    fn blocks_before_onboarding() {
        let mut s = AppSettings::default();
        s.onboarding_complete = false;
        let g = can_start_recording(&s, true, false);
        assert!(!g.allowed);
        assert_eq!(g.reason_key.as_deref(), Some("gate.onboarding"));
    }

    #[test]
    fn local_requires_ready_model() {
        let s = base();
        assert!(!can_start_recording(&s, false, true).allowed);
        assert!(can_start_recording(&s, true, true).allowed);
    }

    #[test]
    fn openrouter_requires_key_and_model() {
        let mut s = base();
        s.stt_provider = SttProvider::OpenRouter;
        s.openrouter_api_key = None;
        assert!(!can_start_recording(&s, false, true).allowed);

        s.openrouter_api_key = Some("sk-or-test".into());
        s.openrouter_stt_model = "openai/gpt-4o-mini-transcribe".into();
        assert!(can_start_recording(&s, false, true).allowed);

        s.openrouter_stt_model = "".into();
        assert!(!can_start_recording(&s, false, true).allowed);
    }
}
