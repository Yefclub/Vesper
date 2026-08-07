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
    /// A server the user runs: Ollama, LM Studio, vLLM, an internal proxy.
    /// Same protocol as OpenRouter, different address and nobody billing.
    #[serde(rename = "openai_compatible", alias = "local_endpoint")]
    OpenAiCompatible,
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
    /// Nothing this application does may reach the network.
    ///
    /// Default off — turning it on for everybody would break the cloud
    /// providers of people who chose them. The switch exists so the promise can
    /// be enforced by somebody who wants it enforced.
    #[serde(default)]
    pub offline_mode: bool,
    /// Ask GitHub at launch whether a newer version exists, and fetch the
    /// installer when one does.
    ///
    /// Defaults on, including for rows written before it existed: an application
    /// that stops telling people about a fix is a worse default than one request
    /// per launch. Offline mode already refused this, but only by refusing
    /// everything — somebody who wants a cloud model and no version check had no
    /// way to say so.
    ///
    /// `#[serde(default = ...)]` is load-bearing for the reason the fields below
    /// already state: `AppState::new` reads the row with `unwrap_or_default()`,
    /// so a field this struct requires and an older row does not carry would
    /// reset every existing user's settings in silence.
    #[serde(default = "default_true")]
    pub auto_update_check: bool,
    /// Base URL of the OpenAI-compatible server, e.g. `http://localhost:11434/v1`.
    /// Checked by `domain::endpoint` before it is used, never on the way in: a
    /// settings file written by an older build must still load.
    #[serde(default)]
    pub endpoint_base_url: String,
    #[serde(default)]
    pub endpoint_model: String,
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
    /// Whether each channel is captured at all — `Me` is the microphone,
    /// `Others` the system audio.
    ///
    /// A switch of its own rather than a third state on the device ids above,
    /// because `None` there already means "the system's default device". See
    /// `domain::channels` for the pair as the recorder receives it.
    ///
    /// `#[serde(default = "default_true")]` on both, and load-bearing for the
    /// reason `recent_openrouter_llm_models` states below: `AppState::new`
    /// reads the row with `unwrap_or_default()`, so a required field an older
    /// row does not carry resets the whole configuration in silence. On, so a
    /// row written before the switches existed records what it always did.
    #[serde(default = "default_true")]
    pub capture_me: bool,
    #[serde(default = "default_true")]
    pub capture_others: bool,
    /// Preferred compute backend: `cpu` | `cuda` | `auto`
    #[serde(default = "default_backend")]
    pub compute_backend: String,
    /// Whether to show the reminder to tell the room they are being recorded.
    /// Defaults on: a meeting tool that records other people should say so at
    /// least once, and the user can turn it off after the first time.
    #[serde(default = "default_true")]
    pub confirm_before_recording: bool,
    /// The global record accelerator, as one of `domain::shortcut::CHOICES`.
    ///
    /// A string like `compute_backend` beside it, with `chosen_or_default` the
    /// one place that decides what an unrecognised value means — a row written
    /// by a build that offered something this one does not must not leave the
    /// user with no shortcut at all.
    #[serde(default = "default_shortcut")]
    pub record_shortcut: String,
    /// Where the floating record card docks: `right_top`, `right_center`,
    /// `right_bottom` or `top`.
    ///
    /// A string rather than the enum, like `compute_backend` beside it: the
    /// WebView sends whatever the select holds, and `OverlayPosition::from_id`
    /// is the one place that decides what an unknown value means.
    #[serde(default = "default_overlay_position")]
    pub overlay_position: String,
    /// Closing the window leaves Vesper running in the notification area
    /// instead of quitting.
    ///
    /// Off by default. A close button that does not close is a surprise, and
    /// the first surprise a new user would meet is an application they cannot
    /// get rid of.
    #[serde(default)]
    pub close_to_tray: bool,
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

    /// Transcribe the whole recording again once it stops.
    ///
    /// Defaults on, including for rows written before it existed: it only ever
    /// runs on the local engine — see `wants_final_stt_pass` — so turning it on
    /// for everybody costs processor time on a machine that is no longer
    /// recording, and nothing else.
    #[serde(default = "default_true")]
    pub final_stt_pass: bool,
    /// Names, products and jargon the engine keeps mishearing.
    ///
    /// Stored already normalised — `domain::vocabulary::normalise` runs on the
    /// way in — so what the field holds is what the prompt will carry, and the
    /// settings screen shows the user the list that is actually in effect.
    #[serde(default)]
    pub hot_words: Vec<String>,

    /// What new meetings call the microphone and the system audio.
    ///
    /// Copied onto a meeting when it is created and never read again: a meeting
    /// keeps the names it was recorded under, so somebody who changes these
    /// before the next call does not rewrite last month's.
    ///
    /// `None` is not a name — it is the absence of one, which leaves the meeting
    /// reading in the app's own words in whatever language the user picks.
    #[serde(default)]
    pub default_speaker_me: Option<String>,
    #[serde(default)]
    pub default_speaker_others: Option<String>,
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
fn default_shortcut() -> String {
    crate::domain::shortcut::RECORD_ACCELERATOR.into()
}
fn default_overlay_position() -> String {
    crate::domain::overlay::OverlayPosition::default()
        .id()
        .into()
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
            offline_mode: false,
            auto_update_check: true,
            // Ollama's own default, which is the server most people already
            // have running.
            endpoint_base_url: "http://localhost:11434/v1".into(),
            endpoint_model: String::new(),
            language: "auto".into(),
            ui_locale: "en".into(),
            onboarding_complete: false,
            mic_device_id: None,
            system_device_id: None,
            capture_me: true,
            capture_others: true,
            compute_backend: "auto".into(),
            confirm_before_recording: true,
            record_shortcut: default_shortcut(),
            overlay_position: default_overlay_position(),
            close_to_tray: false,
            recent_openrouter_llm_models: Vec::new(),
            theme: "light".into(),

            final_stt_pass: true,
            hot_words: Vec::new(),

            default_speaker_me: None,
            default_speaker_others: None,
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

    /// Whether stopping a recording should transcribe the whole of it again.
    ///
    /// The switch, and the provider. A cloud transcriber is deliberately left
    /// out: the live pass already sent every utterance to the same model, so a
    /// second pass over the same audio buys a wider decoding context and is
    /// billed a second time for a meeting the user has already paid to
    /// transcribe. The path that does send a whole recording to a cloud provider
    /// — a stop where no live pass ever ran, an import, a retranscribe — is the
    /// one where nothing was charged for it yet.
    pub fn wants_final_stt_pass(&self) -> bool {
        self.final_stt_pass && self.stt_provider == SttProvider::Local
    }

    pub fn require_openrouter_key(&self) -> Result<(), SettingsError> {
        match &self.openrouter_api_key {
            Some(k) if !k.trim().is_empty() => Ok(()),
            _ => Err(SettingsError::MissingOpenRouterKey),
        }
    }

    /// Move a selection off a model the catalogue no longer offers.
    ///
    /// The catalogue was cut to three tiers per engine, and four of the models
    /// that went were removed over their licence rather than their quality —
    /// somebody is running one right now. Leaving the row pointing at an id
    /// nothing knows about is the worst of the options: the file is still on
    /// disk, `catalog_sha256` answers `None` for it, so it can never be
    /// verified, and the gate refuses to record with a reason naming a model
    /// the picker does not list. A dead end with no way out of it from inside
    /// the app.
    ///
    /// So the pick moves to the nearest surviving tier, and the weights are left
    /// exactly where they are. Deleting gigabytes somebody paid for in bandwidth
    /// because the licence changed under them is not this function's business,
    /// and the file is theirs.
    ///
    /// Returns what was replaced, so the window can say so rather than quietly
    /// summarising with a different model than it did yesterday.
    pub fn migrate_retired_models(&mut self) -> Vec<(String, String)> {
        // Nearest by size within the same engine, which is the axis the user
        // chose on: whoever picked the 3B wanted the big one and gets the new
        // big one, not the smallest thing that still exists.
        // By the tier the user was on rather than by file size. Somebody
        // running the middle model wanted the middle one; landing them on the
        // smallest thing that still exists because it happens to be nearest in
        // megabytes is not the same answer.
        //
        // This is also the only table now. `load_settings` used to run a second
        // one that mapped `qwen2.5-1.5b` onto `llama32-1b` — an id this change
        // retires — so a 1.5B user reached here already renamed and took the
        // wrong exit. Two migrations chained in the wrong order is how a middle
        // tier becomes the smallest one without anybody deciding that.
        const RETIRED: &[(&str, &str)] = &[
            ("whisper-base", "whisper-small"),
            ("llama32-1b", "qwen3-4b-instruct"),
            ("llama32-3b", "qwen3-4b-instruct"),
            ("gemma3-4b", "qwen3-4b-instruct"),
            ("qwen2.5-1.5b", "qwen3-4b-instruct"),
            ("qwen2.5-3b", "mistral-7b-instruct"),
        ];
        let mut moved = Vec::new();
        for (from, to) in RETIRED {
            if self.local_stt_model == *from {
                self.local_stt_model = (*to).into();
                moved.push(((*from).to_string(), (*to).to_string()));
            }
            if self.local_llm_model == *from {
                self.local_llm_model = (*to).into();
                moved.push(((*from).to_string(), (*to).to_string()));
            }
        }
        moved
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

    /// Every retirement has to land on something the catalogue still lists, or
    /// the migration swaps one dead end for another.
    #[test]
    fn every_retirement_lands_on_a_model_that_exists() {
        let offered: Vec<String> = crate::models::list_models()
            .into_iter()
            .map(|m| m.id)
            .collect();
        for from in [
            "whisper-base",
            "llama32-1b",
            "llama32-3b",
            "gemma3-4b",
            "qwen2.5-1.5b",
            "qwen2.5-3b",
        ] {
            let mut s = AppSettings {
                local_stt_model: from.into(),
                local_llm_model: from.into(),
                ..AppSettings::default()
            };
            let moved = s.migrate_retired_models();
            assert!(!moved.is_empty(), "`{from}` was not migrated");
            assert!(
                offered.contains(&s.local_stt_model),
                "stt landed on `{}`, which the catalogue does not offer",
                s.local_stt_model
            );
            assert!(
                offered.contains(&s.local_llm_model),
                "llm landed on `{}`, which the catalogue does not offer",
                s.local_llm_model
            );
        }
    }

    /// A pick the catalogue still offers is left exactly where it is — the
    /// migration must not reshuffle somebody who is happy.
    #[test]
    fn a_current_model_is_not_migrated() {
        let mut s = AppSettings::default();
        let before = (s.local_stt_model.clone(), s.local_llm_model.clone());
        assert!(s.migrate_retired_models().is_empty());
        assert_eq!(
            (s.local_stt_model.clone(), s.local_llm_model.clone()),
            before
        );
    }

    /// The shipped defaults have to be in the catalogue too. A default naming a
    /// model that was dropped is the same dead end, reached by a fresh install
    /// rather than by an upgrade.
    #[test]
    fn the_defaults_are_models_the_catalogue_offers() {
        let offered: Vec<String> = crate::models::list_models()
            .into_iter()
            .map(|m| m.id)
            .collect();
        let s = AppSettings::default();
        assert!(
            offered.contains(&s.local_stt_model),
            "{}",
            s.local_stt_model
        );
        assert!(
            offered.contains(&s.local_llm_model),
            "{}",
            s.local_llm_model
        );
    }

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

    /// The chain that used to exist: `load_settings` renamed `qwen2.5-1.5b` to
    /// `llama32-1b`, and this then had to decide what `llama32-1b` meant. Both
    /// land on the same tier now, whichever door the row came through.
    #[test]
    fn the_middle_tier_stays_the_middle_tier() {
        for from in ["qwen2.5-1.5b", "llama32-1b", "llama32-3b", "gemma3-4b"] {
            let mut s = AppSettings {
                local_llm_model: from.into(),
                ..AppSettings::default()
            };
            s.migrate_retired_models();
            assert_eq!(s.local_llm_model, "qwen3-4b-instruct", "from `{from}`");
        }
    }

    #[test]
    fn the_biggest_stays_the_biggest() {
        let mut s = AppSettings {
            local_llm_model: "qwen2.5-3b".into(),
            ..AppSettings::default()
        };
        s.migrate_retired_models();
        assert_eq!(s.local_llm_model, "mistral-7b-instruct");
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
        assert_eq!(s.default_speaker_me, None);
        assert_eq!(s.default_speaker_others, None);
        // Both channels, which is the only reading of a row written before
        // there was a way to turn one off.
        assert!(s.capture_me);
        assert!(s.capture_others);
    }

    /// A channel switched off survives a save and a load. The pair is written
    /// into the same JSON blob as everything else, so a field serde could not
    /// read back would take the rest of the configuration down with it.
    #[test]
    fn a_channel_switched_off_roundtrips_serde() {
        let s = AppSettings {
            capture_others: false,
            ..Default::default()
        };
        let s2: AppSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(s2.capture_me);
        assert!(!s2.capture_others);
    }

    /// A row written before the field existed still has to load, and has to load
    /// with the pass ON — a local user upgrading gets the better transcript
    /// without going looking for a checkbox.
    #[test]
    fn the_final_pass_is_on_for_a_row_that_predates_it() {
        let older_row = r#"{
            "stt_provider": "local",
            "llm_provider": "local",
            "openrouter_api_key": null,
            "openrouter_stt_model": "openai/gpt-4o-mini-transcribe",
            "openrouter_llm_model": "openai/gpt-4o-mini",
            "local_stt_model": "whisper-small",
            "local_llm_model": "llama32-1b",
            "reasoning_enabled": false,
            "auto_summarize": true,
            "language": "pt",
            "ui_locale": "pt-BR",
            "onboarding_complete": true,
            "compute_backend": "auto",
            "confirm_before_recording": true
        }"#;
        let s: AppSettings = serde_json::from_str(older_row).expect("older rows must still load");
        assert!(s.final_stt_pass);
        assert!(s.hot_words.is_empty());
        assert!(s.wants_final_stt_pass());
    }

    /// A fresh install checks, and so does a row written before the switch
    /// existed — with the rest of that row intact. The field is required by the
    /// struct, so without its `serde` default the whole row fails to
    /// deserialise, and `AppState::new` turns that failure into a factory-fresh
    /// configuration: cloud provider forgotten, devices forgotten, language back
    /// to English, and nothing on screen saying why.
    #[test]
    fn the_update_check_is_on_for_a_row_that_predates_it() {
        assert!(AppSettings::default().auto_update_check);
        let older_row = r#"{
            "stt_provider": "local",
            "llm_provider": "openrouter",
            "openrouter_api_key": null,
            "openrouter_stt_model": "openai/gpt-4o-mini-transcribe",
            "openrouter_llm_model": "openai/gpt-4o-mini",
            "local_stt_model": "whisper-small",
            "local_llm_model": "qwen3-4b-instruct",
            "reasoning_enabled": false,
            "auto_summarize": true,
            "language": "pt",
            "ui_locale": "pt-BR",
            "onboarding_complete": true,
            "mic_device_id": "mic-1",
            "compute_backend": "auto",
            "confirm_before_recording": true
        }"#;
        let s: AppSettings = serde_json::from_str(older_row).expect("older rows must still load");
        assert!(s.auto_update_check);
        assert_eq!(s.llm_provider, LlmProvider::OpenRouter);
        assert_eq!(s.ui_locale, "pt-BR");
        assert_eq!(s.mic_device_id.as_deref(), Some("mic-1"));
        assert!(s.onboarding_complete);
    }

    /// And a row that carries it off keeps it off — the one state a default of
    /// `true` could quietly overwrite.
    #[test]
    fn a_row_that_turned_the_update_check_off_keeps_it_off() {
        let s = AppSettings {
            auto_update_check: false,
            ..AppSettings::default()
        };
        let back: AppSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.auto_update_check);
    }

    /// Off means off: a meeting stopped with the switch down does exactly what
    /// it did before this existed.
    #[test]
    fn the_switch_alone_can_turn_the_final_pass_off() {
        let s = AppSettings {
            final_stt_pass: false,
            ..AppSettings::default()
        };
        assert!(!s.wants_final_stt_pass());
    }

    /// And a cloud transcriber never gets one, however the switch is set — the
    /// audio would be sent, and billed, a second time.
    #[test]
    fn a_cloud_transcriber_never_runs_the_final_pass() {
        let s = AppSettings {
            stt_provider: SttProvider::OpenRouter,
            final_stt_pass: true,
            ..AppSettings::default()
        };
        assert!(!s.wants_final_stt_pass());
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
