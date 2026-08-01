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
    can_start_recording_with(settings, local_stt_ready, false, onboarding_complete)
}

/// `local_stt_present` separates "the file is missing" from "the file is there but
/// this build has never verified it". Saying a model is not installed while it sits
/// on the user's disk sends them to re-download something they already have — and
/// the settings screen, which knows the difference, would be contradicting this one.
pub fn can_start_recording_with(
    settings: &AppSettings,
    local_stt_ready: bool,
    local_stt_present: bool,
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
            } else if local_stt_present {
                StartGate {
                    allowed: false,
                    reason: Some(format!(
                        "Local STT model `{}` is on disk but has not been verified yet.",
                        settings.local_stt_model
                    )),
                    reason_key: Some("gate.local_stt_unverified".into()),
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
        AppSettings {
            onboarding_complete: true,
            ..Default::default()
        }
    }

    #[test]
    fn blocks_before_onboarding() {
        let s = AppSettings {
            onboarding_complete: false,
            ..Default::default()
        };
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
