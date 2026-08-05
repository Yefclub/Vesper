use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Meeting / processing lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Idle,
    Recording,
    Paused,
    Transcribing,
    Summarizing,
    Ready,
    Failed,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("invalid transition from {from:?} via {event:?}")]
    Invalid {
        from: MeetingStatus,
        event: MeetingEvent,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingEvent {
    StartRecording,
    Pause,
    Resume,
    StopRecording,
    StartTranscribe,
    TranscribeDone,
    StartSummarize,
    SummarizeDone,
    Fail,
    Reset,
}

impl MeetingStatus {
    pub fn transition(self, event: MeetingEvent) -> Result<MeetingStatus, TransitionError> {
        use MeetingEvent::*;
        use MeetingStatus::*;
        let next = match (self, event) {
            (Idle, StartRecording) => Recording,
            (Recording, Pause) => Paused,
            (Paused, Resume) => Recording,
            (Recording, StopRecording) | (Paused, StopRecording) => Transcribing,
            (Transcribing, TranscribeDone) => Ready,
            (Transcribing, StartSummarize) => Summarizing,
            (Ready, StartSummarize) => Summarizing,
            (Summarizing, SummarizeDone) => Ready,
            (Ready, StartTranscribe) => Transcribing,
            (_, Fail) if !matches!(self, Idle) => Failed,
            (Failed, Reset) | (Ready, Reset) => Idle,
            (Failed, StartRecording) => Recording,
            // Allow re-transcribe from ready/failed
            (Failed, StartTranscribe) => Transcribing,
            _ => return Err(TransitionError::Invalid { from: self, event }),
        };
        Ok(next)
    }

    pub fn is_capturing(self) -> bool {
        matches!(self, MeetingStatus::Recording)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingRecord {
    pub id: String,
    pub title: String,
    pub status: MeetingStatus,
    pub created_at: String,
    pub updated_at: String,
    pub duration_ms: u64,
    pub audio_path: Option<String>,
    pub transcript_text: String,
    pub summary: Option<String>,
    pub action_items: Option<String>,
    pub key_points: Option<String>,
    /// Every section the template asked for, in order.
    ///
    /// Empty for a meeting summarised before templates had shapes of their own,
    /// and for one never summarised at all. The screen falls back to the three
    /// fields above in that case, which are still written for every template
    /// that has them — the search index, the exporter and the chat context all
    /// read those by name.
    #[serde(default)]
    pub sections: Vec<crate::domain::summary::SummarySection>,
    pub project: Option<String>,
    /// The user named this meeting, so nothing generated may replace it.
    ///
    /// Provenance, not inference. Recognising the fallback by its shape works
    /// until someone renames a meeting to something that happens to have that
    /// shape — `Meeting 2026-08-01 10:30` is a perfectly ordinary thing to type,
    /// and the next summary would silently take it back.
    ///
    /// `#[serde(default)]` because a row written before this column existed
    /// deserialises through here, and settings and meetings alike are loaded with
    /// a fallback that would swallow the error and lose the row.
    #[serde(default)]
    pub title_locked: bool,
    /// What this meeting has cost in provider charges, in nano-USD.
    ///
    /// `None` means it never called a paid provider — a locally transcribed and
    /// locally summarised meeting shows no price at all, which is a different
    /// statement from `Some(0)` for one that ran on a free model.
    #[serde(default)]
    pub cost_nano_usd: Option<i64>,
    /// The same figure as the window shows it. Derived on read, never stored:
    /// money formatting is one decision and it belongs on the side that owns
    /// the number.
    #[serde(default)]
    pub cost_label: Option<String>,
}

/// What the app is doing to a meeting after Stop, as the window renders it.
///
/// Not a second state machine — `MeetingStatus` above stays the one that drives
/// the row. This is the wire vocabulary for `meeting://progress`, which needs
/// two states the row has no use for: the moment before transcription starts,
/// and a summary that failed without costing the user the recording.
///
/// There is deliberately no percentage. Whisper is one `spawn_blocking` over the
/// whole buffer and the cloud completion is one non-streamed POST — there is no
/// progress to report, and a named phase is the honest maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingPhase {
    Saving,
    Transcribing,
    /// The whole recording being transcribed again, after the live pass.
    ///
    /// Its own phase rather than a second `Transcribing`: the two are different
    /// work on different audio — one caught up with the last utterance, this one
    /// reads the meeting from the beginning — and a window that cannot tell them
    /// apart shows the same spinner for seconds and then for minutes.
    FinalPass,
    Summarizing,
    Ready,
    SummaryFailed,
    /// The whole-recording pass did not finish.
    ///
    /// Not a failure of the meeting: the live transcript is saved and the row is
    /// already `Ready` before this pass is attempted, so what this reports is an
    /// improvement that did not arrive. Like `SummaryFailed`, it names an outcome
    /// the walk is over for, not a step in it.
    FinalPassFailed,
}

/// Payload of `meeting://progress`. One channel, so the window can tell that a
/// later phase supersedes an earlier one — separate channels give no ordering
/// guarantee across the WebView boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MeetingProgress {
    pub meeting_id: String,
    pub phase: MeetingPhase,
    pub error: Option<String>,
}

impl MeetingProgress {
    pub fn new(meeting_id: &str, phase: MeetingPhase) -> Self {
        Self {
            meeting_id: meeting_id.to_string(),
            phase,
            error: None,
        }
    }

    pub fn summary_failed(meeting_id: &str, error: &str) -> Self {
        Self {
            meeting_id: meeting_id.to_string(),
            phase: MeetingPhase::SummaryFailed,
            error: Some(error.to_string()),
        }
    }

    pub fn final_pass_failed(meeting_id: &str, error: &str) -> Self {
        Self {
            meeting_id: meeting_id.to_string(),
            phase: MeetingPhase::FinalPassFailed,
            error: Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_recording_to_ready() {
        let mut s = MeetingStatus::Idle;
        s = s.transition(MeetingEvent::StartRecording).unwrap();
        assert_eq!(s, MeetingStatus::Recording);
        s = s.transition(MeetingEvent::Pause).unwrap();
        assert_eq!(s, MeetingStatus::Paused);
        s = s.transition(MeetingEvent::Resume).unwrap();
        assert_eq!(s, MeetingStatus::Recording);
        s = s.transition(MeetingEvent::StopRecording).unwrap();
        assert_eq!(s, MeetingStatus::Transcribing);
        s = s.transition(MeetingEvent::TranscribeDone).unwrap();
        assert_eq!(s, MeetingStatus::Ready);
    }

    #[test]
    fn summarize_from_ready() {
        let s = MeetingStatus::Ready
            .transition(MeetingEvent::StartSummarize)
            .unwrap();
        assert_eq!(s, MeetingStatus::Summarizing);
        let s = s.transition(MeetingEvent::SummarizeDone).unwrap();
        assert_eq!(s, MeetingStatus::Ready);
    }

    #[test]
    fn invalid_transition_errors() {
        let err = MeetingStatus::Idle
            .transition(MeetingEvent::Pause)
            .unwrap_err();
        assert!(matches!(err, TransitionError::Invalid { .. }));
    }

    #[test]
    fn every_phase_keeps_its_wire_name() {
        // `#[serde(rename_all)]` is the entire API between the two halves of the
        // app and nothing type-checks across it: the window switches on these
        // exact strings, so a rename here is a spinner that never stops there.
        for (phase, wire) in [
            (MeetingPhase::Saving, "saving"),
            (MeetingPhase::Transcribing, "transcribing"),
            (MeetingPhase::FinalPass, "final_pass"),
            (MeetingPhase::Summarizing, "summarizing"),
            (MeetingPhase::Ready, "ready"),
            (MeetingPhase::SummaryFailed, "summary_failed"),
            (MeetingPhase::FinalPassFailed, "final_pass_failed"),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), wire);
        }
    }

    #[test]
    fn a_progress_payload_keeps_its_field_names() {
        // Same contract, one level up: the phase can be named correctly and
        // still be unreachable if the key it arrives under drifts.
        let v = serde_json::to_value(MeetingProgress::summary_failed("m-1", "no model")).unwrap();
        assert_eq!(v["meeting_id"], "m-1");
        assert_eq!(v["phase"], "summary_failed");
        assert_eq!(v["error"], "no model");
        assert_eq!(
            serde_json::to_value(MeetingProgress::new("m-1", MeetingPhase::Saving)).unwrap()
                ["error"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn fail_and_reset() {
        let s = MeetingStatus::Recording
            .transition(MeetingEvent::Fail)
            .unwrap();
        assert_eq!(s, MeetingStatus::Failed);
        let s = s.transition(MeetingEvent::Reset).unwrap();
        assert_eq!(s, MeetingStatus::Idle);
    }
}
