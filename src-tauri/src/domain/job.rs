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
            _ => {
                return Err(TransitionError::Invalid {
                    from: self,
                    event,
                })
            }
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
    pub project: Option<String>,
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
    fn fail_and_reset() {
        let s = MeetingStatus::Recording
            .transition(MeetingEvent::Fail)
            .unwrap();
        assert_eq!(s, MeetingStatus::Failed);
        let s = s.transition(MeetingEvent::Reset).unwrap();
        assert_eq!(s, MeetingStatus::Idle);
    }
}
