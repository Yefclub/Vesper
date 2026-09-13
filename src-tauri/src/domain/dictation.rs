//! Dictation — speaking into whichever application has the keyboard.
//!
//! Pure, like the rest of `domain`: which state a session is in and what may
//! follow it, whether one may start, whether the window a transcript was aimed
//! at is still the one it would land in, and what is recorded about the attempt.
//! Capture, transcription and the per-platform code that types into other
//! applications live outside and report into these types.

use crate::domain::channels::ChannelSelection;
use crate::domain::gate::can_start_recording_with;
use crate::domain::settings::{AppSettings, SttProvider};
use serde::{Deserialize, Serialize};

/// What a dictation records: the microphone, never the system audio.
///
/// A constant rather than the meeting switches. Those decide what a meeting
/// hears; a dictation is the user talking to their own keyboard, and whatever
/// the speakers are playing — a call, a video — is not theirs to type.
pub const CAPTURE: ChannelSelection = ChannelSelection {
    me: true,
    others: false,
};

/// Where a dictation is.
///
/// Stored by name in the history, so a session interrupted by a crash reads
/// after the restart as the state `recovered` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationState {
    /// Nothing in progress. Reported to the window, never stored: a history row
    /// starts at `Listening`.
    Idle,
    Listening,
    Transcribing,
    Inserting,
    /// The text is kept and was not typed anywhere: nothing was said, or there
    /// was no target this platform can type into.
    Saved,
    /// Typed into the target, and the target read back with it.
    Inserted,
    /// Typed into the target, which could not be read back to confirm it.
    Unconfirmed,
    /// Not typed: the target was gone, had changed, refused, or was out of reach.
    InsertionFailed,
    /// The transcriber failed. What it had already accepted is kept, and so is
    /// the audio, for a retry.
    TranscriptionFailed,
}

impl DictationState {
    /// Still holding the microphone, the transcriber or the keyboard: a second
    /// dictation cannot start until this one lets go.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            DictationState::Listening | DictationState::Transcribing | DictationState::Inserting
        )
    }
}

/// Why text did not go in.
///
/// A closed list, stored and shown by name. A free-form message from a platform
/// API could carry a window title or a field's contents into the history and
/// the screen, and neither is ours to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// The keyboard moved to another window or control after dictation started.
    TargetChanged,
    /// The window it started in is gone.
    TargetClosed,
    /// Nothing focused accepts text, or the focus is in Vesper itself.
    NoEditableTarget,
    /// The target runs with more privilege than Vesper, and the system discards
    /// input sent to it without saying so.
    ElevatedTarget,
    /// The permission to type into other applications has not been granted.
    PermissionDenied,
    /// A password field or similar has taken exclusive hold of the keyboard.
    SecureInput,
    /// This desktop gives applications no way to type into others.
    Unsupported,
    Timeout,
    /// Using the clipboard would have lost what the user had on it.
    ClipboardUnsafe,
    /// The application quit while this dictation was being typed.
    Interrupted,
    /// The platform refused without a reason this list can name.
    Failed,
}

impl FailureReason {
    /// The i18n key the window shows for it.
    pub fn key(self) -> &'static str {
        match self {
            FailureReason::TargetChanged => "dictation.failure.target_changed",
            FailureReason::TargetClosed => "dictation.failure.target_closed",
            FailureReason::NoEditableTarget => "dictation.failure.no_editable_target",
            FailureReason::ElevatedTarget => "dictation.failure.elevated_target",
            FailureReason::PermissionDenied => "dictation.failure.permission_denied",
            FailureReason::SecureInput => "dictation.failure.secure_input",
            FailureReason::Unsupported => "dictation.failure.unsupported",
            FailureReason::Timeout => "dictation.failure.timeout",
            FailureReason::ClipboardUnsafe => "dictation.failure.clipboard_unsafe",
            FailureReason::Interrupted => "dictation.failure.interrupted",
            FailureReason::Failed => "dictation.failure.failed",
        }
    }
}

/// What an insertion attempt concluded. Only `Inserted` claims the text is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertionOutcome {
    Inserted,
    Unconfirmed,
    Failed(FailureReason),
}

/// Something that happened to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationEvent {
    Stop,
    /// Transcription finished. `will_insert` is false when there is no target
    /// this platform can type into, so the text is only kept.
    Transcribed {
        has_text: bool,
        will_insert: bool,
    },
    TranscriptionFailed,
    Insertion(InsertionOutcome),
    RetryInsertion,
    RetryTranscription,
}

/// Why an event does not apply to the state a session is in.
///
/// A second press of the shortcut, a stop that arrives after the first one, or
/// an insertion result replayed for a session already concluded all land here
/// instead of acting twice — and acting twice is typing the text twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    NotListening,
    NotTranscribing,
    NotInserting,
    NotRetryable,
}

/// The state a session moves to, or why it does not.
pub fn next(state: DictationState, event: DictationEvent) -> Result<DictationState, Refused> {
    use DictationEvent as E;
    use DictationState as S;
    match (state, event) {
        (S::Listening, E::Stop) => Ok(S::Transcribing),
        (_, E::Stop) => Err(Refused::NotListening),

        (
            S::Transcribing,
            E::Transcribed {
                has_text: true,
                will_insert: true,
            },
        ) => Ok(S::Inserting),
        (S::Transcribing, E::Transcribed { .. }) => Ok(S::Saved),
        (S::Transcribing, E::TranscriptionFailed) => Ok(S::TranscriptionFailed),
        (_, E::Transcribed { .. } | E::TranscriptionFailed) => Err(Refused::NotTranscribing),

        (S::Inserting, E::Insertion(outcome)) => Ok(match outcome {
            InsertionOutcome::Inserted => S::Inserted,
            InsertionOutcome::Unconfirmed => S::Unconfirmed,
            InsertionOutcome::Failed(_) => S::InsertionFailed,
        }),
        (_, E::Insertion(_)) => Err(Refused::NotInserting),

        // `Unconfirmed` is retryable because only the user can see whether the
        // text arrived, and the retry is theirs to ask for. It never happens by
        // itself.
        (S::Saved | S::InsertionFailed | S::Unconfirmed, E::RetryInsertion) => Ok(S::Inserting),
        (S::TranscriptionFailed, E::RetryTranscription) => Ok(S::Transcribing),
        (_, E::RetryInsertion | E::RetryTranscription) => Err(Refused::NotRetryable),
    }
}

/// What a session found mid-flight at launch becomes. The process that owned it
/// is gone.
///
/// Nothing already accepted is dropped. A take whose audio is still on disk is
/// a transcription to retry; with text but no audio, the text is the dictation;
/// with neither there was nothing to keep, and `None` says to delete the row.
/// An insertion never resumes: typing into whatever has the keyboard now is
/// exactly what the target check exists to prevent.
pub fn recovered(
    state: DictationState,
    has_audio: bool,
    has_text: bool,
) -> Option<(DictationState, Option<FailureReason>)> {
    match state {
        DictationState::Listening | DictationState::Transcribing => {
            if has_audio {
                Some((DictationState::TranscriptionFailed, None))
            } else if has_text {
                Some((DictationState::Saved, None))
            } else {
                None
            }
        }
        DictationState::Inserting => Some((
            DictationState::InsertionFailed,
            Some(FailureReason::Interrupted),
        )),
        concluded => Some((concluded, None)),
    }
}

/// The application and control the keyboard belonged to when dictation started.
///
/// Opaque numbers only — a window handle, a control handle, a process id and its
/// start time — held in memory for one session and never written or logged. A
/// title or a field's contents would say what the user was doing, and nothing
/// here needs to know that to tell whether the window is still the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub window: u64,
    /// `0` where the platform cannot name the focused control.
    pub control: u64,
    pub process: u32,
    /// `0` where the platform cannot say. Otherwise it tells a program apart from
    /// a later one that was handed the same process id.
    pub process_started: u64,
}

/// Whether text aimed at `captured` may go into what has the keyboard `now`.
///
/// Vesper's own windows are never a target: the indicator and the main window
/// hold no field a dictation is meant for, and typing into them would be typing
/// nowhere while reporting somewhere.
pub fn still_the_target(
    captured: Target,
    now: Option<Target>,
    own_process: u32,
) -> Result<(), FailureReason> {
    if captured.process == own_process {
        return Err(FailureReason::NoEditableTarget);
    }
    let Some(now) = now else {
        return Err(FailureReason::TargetClosed);
    };
    if now.process != captured.process
        || now.process_started != captured.process_started
        || now.window != captured.window
    {
        return Err(FailureReason::TargetChanged);
    }
    // An unknown control at the start cannot be compared, and refusing on it
    // would refuse every application that does not expose one.
    if captured.control != 0 && now.control != captured.control {
        return Err(FailureReason::TargetChanged);
    }
    Ok(())
}

/// The text a session inserts: its accepted segments in order, one space apart.
///
/// Trimmed piece by piece because the transcriber leads most segments with a
/// space of its own, and those doubled up between segments.
pub fn joined<'a>(segments: impl IntoIterator<Item = &'a str>) -> String {
    segments
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A dictation as the history lists it.
///
/// `text` is the joined segments rather than a column of its own, so what is
/// shown is exactly what was accepted — there is no second copy to fall out of
/// step when a segment lands just before a crash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DictationRecord {
    pub id: String,
    pub created_at: String,
    pub state: DictationState,
    pub failure: Option<FailureReason>,
    pub duration_ms: i64,
    pub text: String,
    /// Whether a failed transcription still has its audio to retry from.
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DictationGate {
    pub allowed: bool,
    pub reason_key: Option<String>,
}

/// Whether a dictation may start.
///
/// A meeting in progress holds the microphone and the transcriber, and the two
/// never run at once. The transcriber checks are the recording gate's own, so a
/// missing model reads the same in both places; the cloud has two refusals of
/// its own on top, because a shortcut pressed in another application is not the
/// same decision as a meeting the user starts in this one.
pub fn can_start(
    settings: &AppSettings,
    meeting_active: bool,
    dictation_active: bool,
    local_stt_ready: bool,
) -> DictationGate {
    let refuse = |key: &str| DictationGate {
        allowed: false,
        reason_key: Some(key.into()),
    };
    if dictation_active {
        return refuse("dictation.gate.active");
    }
    if meeting_active {
        return refuse("dictation.gate.meeting");
    }
    if settings.stt_provider == SttProvider::OpenRouter {
        if settings.offline_mode {
            return refuse("dictation.gate.offline");
        }
        if !settings.dictation_cloud_consent {
            return refuse("dictation.gate.cloud_consent");
        }
    }
    // With the microphone switched on: a meeting switch turned off says nothing
    // about dictation, which only ever records the microphone.
    let transcriber = can_start_recording_with(
        &AppSettings {
            capture_me: true,
            ..settings.clone()
        },
        local_stt_ready,
        false,
        true,
    );
    DictationGate {
        allowed: transcriber.allowed,
        reason_key: transcriber.reason_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 7;

    fn target() -> Target {
        Target {
            window: 0x10,
            control: 0x20,
            process: 42,
            process_started: 1_000,
        }
    }

    fn ready() -> AppSettings {
        AppSettings {
            onboarding_complete: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_dictation_never_captures_system_audio() {
        assert!(CAPTURE.me);
        assert!(!CAPTURE.others);
    }

    #[test]
    fn a_session_runs_listening_to_inserted() {
        let s = next(DictationState::Listening, DictationEvent::Stop).unwrap();
        assert_eq!(s, DictationState::Transcribing);
        let s = next(
            s,
            DictationEvent::Transcribed {
                has_text: true,
                will_insert: true,
            },
        )
        .unwrap();
        assert_eq!(s, DictationState::Inserting);
        let s = next(s, DictationEvent::Insertion(InsertionOutcome::Inserted)).unwrap();
        assert_eq!(s, DictationState::Inserted);
    }

    /// A second press, a late stop or a replayed result must not act again —
    /// acting twice on an insertion is typing the text twice.
    #[test]
    fn duplicates_are_refused() {
        assert_eq!(
            next(DictationState::Transcribing, DictationEvent::Stop),
            Err(Refused::NotListening)
        );
        assert_eq!(
            next(DictationState::Idle, DictationEvent::Stop),
            Err(Refused::NotListening)
        );
        assert_eq!(
            next(
                DictationState::Inserted,
                DictationEvent::Insertion(InsertionOutcome::Inserted)
            ),
            Err(Refused::NotInserting)
        );
        assert_eq!(
            next(
                DictationState::Saved,
                DictationEvent::Transcribed {
                    has_text: true,
                    will_insert: true
                }
            ),
            Err(Refused::NotTranscribing)
        );
    }

    #[test]
    fn nothing_said_or_no_target_is_saved_not_inserted() {
        for (has_text, will_insert) in [(false, true), (true, false), (false, false)] {
            assert_eq!(
                next(
                    DictationState::Transcribing,
                    DictationEvent::Transcribed {
                        has_text,
                        will_insert
                    }
                ),
                Ok(DictationState::Saved)
            );
        }
    }

    #[test]
    fn only_a_confirmed_insertion_is_inserted() {
        assert_eq!(
            next(
                DictationState::Inserting,
                DictationEvent::Insertion(InsertionOutcome::Unconfirmed)
            ),
            Ok(DictationState::Unconfirmed)
        );
        assert_eq!(
            next(
                DictationState::Inserting,
                DictationEvent::Insertion(InsertionOutcome::Failed(FailureReason::TargetChanged))
            ),
            Ok(DictationState::InsertionFailed)
        );
    }

    #[test]
    fn retries_apply_only_where_they_mean_something() {
        for s in [
            DictationState::Saved,
            DictationState::InsertionFailed,
            DictationState::Unconfirmed,
        ] {
            assert_eq!(
                next(s, DictationEvent::RetryInsertion),
                Ok(DictationState::Inserting)
            );
        }
        assert_eq!(
            next(DictationState::Inserted, DictationEvent::RetryInsertion),
            Err(Refused::NotRetryable)
        );
        assert_eq!(
            next(
                DictationState::TranscriptionFailed,
                DictationEvent::RetryTranscription
            ),
            Ok(DictationState::Transcribing)
        );
        assert_eq!(
            next(
                DictationState::Listening,
                DictationEvent::RetryTranscription
            ),
            Err(Refused::NotRetryable)
        );
    }

    #[test]
    fn only_listening_transcribing_and_inserting_hold_the_session() {
        assert!(DictationState::Listening.is_active());
        assert!(DictationState::Transcribing.is_active());
        assert!(DictationState::Inserting.is_active());
        for s in [
            DictationState::Idle,
            DictationState::Saved,
            DictationState::Inserted,
            DictationState::Unconfirmed,
            DictationState::InsertionFailed,
            DictationState::TranscriptionFailed,
        ] {
            assert!(!s.is_active(), "{s:?}");
        }
    }

    /// A crash must not cost what was already accepted, and must never resume
    /// typing into whatever has the keyboard after the restart.
    #[test]
    fn recovery_keeps_what_was_accepted_and_never_resumes_typing() {
        assert_eq!(
            recovered(DictationState::Listening, true, true),
            Some((DictationState::TranscriptionFailed, None))
        );
        assert_eq!(
            recovered(DictationState::Transcribing, false, true),
            Some((DictationState::Saved, None))
        );
        assert_eq!(recovered(DictationState::Listening, false, false), None);
        assert_eq!(
            recovered(DictationState::Inserting, false, true),
            Some((
                DictationState::InsertionFailed,
                Some(FailureReason::Interrupted)
            ))
        );
        assert_eq!(
            recovered(DictationState::Inserted, false, true),
            Some((DictationState::Inserted, None))
        );
    }

    #[test]
    fn the_same_window_and_control_is_still_the_target() {
        assert_eq!(still_the_target(target(), Some(target()), OWN), Ok(()));
    }

    #[test]
    fn any_change_of_window_control_or_program_refuses() {
        let t = target();
        for now in [
            Target { window: 0x11, ..t },
            Target { control: 0x21, ..t },
            Target { process: 43, ..t },
            // A program that quit and a new one handed the same pid.
            Target {
                process_started: 2_000,
                ..t
            },
        ] {
            assert_eq!(
                still_the_target(t, Some(now), OWN),
                Err(FailureReason::TargetChanged),
                "{now:?}"
            );
        }
        assert_eq!(
            still_the_target(t, None, OWN),
            Err(FailureReason::TargetClosed)
        );
    }

    #[test]
    fn an_unknown_control_is_not_compared() {
        let t = Target {
            control: 0,
            ..target()
        };
        assert_eq!(
            still_the_target(t, Some(Target { control: 0x99, ..t }), OWN),
            Ok(())
        );
    }

    #[test]
    fn vesper_itself_is_never_a_target() {
        let own = Target {
            process: OWN,
            ..target()
        };
        assert_eq!(
            still_the_target(own, Some(own), OWN),
            Err(FailureReason::NoEditableTarget)
        );
    }

    #[test]
    fn segments_join_without_doubled_spaces() {
        assert_eq!(
            joined([" Hello", " world. ", "", " Again"]),
            "Hello world. Again"
        );
        assert_eq!(joined(Vec::<&str>::new()), "");
    }

    /// Stored and shown by name: the snake_case spelling is the contract with the
    /// history table and the window.
    #[test]
    fn states_and_reasons_keep_their_stored_names() {
        assert_eq!(
            serde_json::to_value(DictationState::InsertionFailed).unwrap(),
            serde_json::json!("insertion_failed")
        );
        assert_eq!(
            serde_json::to_value(FailureReason::TargetChanged).unwrap(),
            serde_json::json!("target_changed")
        );
        assert_eq!(
            FailureReason::TargetChanged.key(),
            "dictation.failure.target_changed"
        );
    }

    #[test]
    fn a_meeting_in_progress_refuses_dictation() {
        let g = can_start(&ready(), true, false, true);
        assert!(!g.allowed);
        assert_eq!(g.reason_key.as_deref(), Some("dictation.gate.meeting"));
    }

    #[test]
    fn an_active_dictation_refuses_another() {
        let g = can_start(&ready(), false, true, true);
        assert_eq!(g.reason_key.as_deref(), Some("dictation.gate.active"));
    }

    #[test]
    fn local_dictation_needs_the_model_and_nothing_else() {
        assert!(can_start(&ready(), false, false, true).allowed);
        let g = can_start(&ready(), false, false, false);
        assert!(!g.allowed);
        assert_eq!(g.reason_key.as_deref(), Some("gate.local_stt"));
    }

    /// Meeting switches say what a meeting hears. A microphone turned off there
    /// is not a reason to refuse dictation, which records nothing else.
    #[test]
    fn meeting_channel_switches_do_not_block_dictation() {
        let s = AppSettings {
            capture_me: false,
            capture_others: false,
            ..ready()
        };
        assert!(can_start(&s, false, false, true).allowed);
    }

    #[test]
    fn cloud_dictation_needs_its_own_consent_and_the_network() {
        let mut s = ready();
        s.stt_provider = SttProvider::OpenRouter;
        s.openrouter_api_key = Some("sk-or-test".into());
        assert_eq!(
            can_start(&s, false, false, false).reason_key.as_deref(),
            Some("dictation.gate.cloud_consent")
        );
        s.dictation_cloud_consent = true;
        assert!(can_start(&s, false, false, false).allowed);
        s.offline_mode = true;
        assert_eq!(
            can_start(&s, false, false, false).reason_key.as_deref(),
            Some("dictation.gate.offline")
        );
    }

    #[test]
    fn cloud_dictation_still_needs_the_key() {
        let mut s = ready();
        s.stt_provider = SttProvider::OpenRouter;
        s.dictation_cloud_consent = true;
        s.openrouter_api_key = None;
        assert_eq!(
            can_start(&s, false, false, false).reason_key.as_deref(),
            Some("gate.openrouter_key")
        );
    }
}
