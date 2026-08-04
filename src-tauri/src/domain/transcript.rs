use crate::domain::speaker::Speaker;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub id: String,
    pub speaker: Speaker,
    pub text: String,
    /// Start offset in milliseconds from meeting start.
    pub start_ms: u64,
    /// End offset in milliseconds from meeting start.
    pub end_ms: u64,
}

impl TranscriptSegment {
    pub fn new(speaker: Speaker, text: impl Into<String>, start_ms: u64, end_ms: u64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            speaker,
            text: text.into(),
            start_ms,
            end_ms: end_ms.max(start_ms),
        }
    }
}

/// Ordered live transcript buffer. Append is chronological by start_ms;
/// ties break by insertion order within the same start.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LiveTranscript {
    pub segments: Vec<TranscriptSegment>,
}

impl LiveTranscript {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    pub fn append(&mut self, segment: TranscriptSegment) {
        let text = segment.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut seg = segment;
        seg.text = text;
        // Keep sorted by start_ms; stable insert for equal starts.
        match self
            .segments
            .binary_search_by_key(&seg.start_ms, |s| s.start_ms)
        {
            Ok(mut idx) => {
                // Find end of equal start_ms run and insert after.
                while idx < self.segments.len() && self.segments[idx].start_ms == seg.start_ms {
                    idx += 1;
                }
                self.segments.insert(idx, seg);
            }
            Err(idx) => self.segments.insert(idx, seg),
        }
    }

    pub fn segments(&self) -> &[TranscriptSegment] {
        &self.segments
    }

    pub fn plain_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| format!("{}: {}", s.speaker.label(), s.text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The same lines, keeping the clock.
    ///
    /// `plain_text` drops the timestamps, which is right for a first summary —
    /// they are noise when the task is "what happened". A refinement is being
    /// asked to find what the first pass missed, and when something was said is
    /// most of how a model tells a decision from an aside.
    pub fn timestamped_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| {
                let secs = s.start_ms / 1000;
                format!(
                    "[{:02}:{:02}] {}: {}",
                    secs / 60,
                    secs % 60,
                    s.speaker.label(),
                    s.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn merge_from(other: &LiveTranscript) -> LiveTranscript {
        let mut t = LiveTranscript::new();
        for s in other.segments() {
            t.append(s.clone());
        }
        t
    }
}

/// Format milliseconds as [mm:ss] for display.
pub fn format_ts(ms: u64) -> String {
    let total_secs = ms / 1000;
    let m = total_secs / 60;
    let s = total_secs % 60;
    format!("[{m:02}:{s:02}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_orders_by_start_ms() {
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(
            Speaker::Others,
            "second",
            2000,
            3000,
        ));
        t.append(TranscriptSegment::new(Speaker::Me, "first", 0, 1000));
        t.append(TranscriptSegment::new(Speaker::Me, "mid", 1000, 2000));
        let texts: Vec<_> = t.segments().iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["first", "mid", "second"]);
    }

    #[test]
    fn rejects_empty_text() {
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(Speaker::Me, "   ", 0, 10));
        assert!(t.segments().is_empty());
    }

    #[test]
    fn plain_text_labels_speakers() {
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(Speaker::Me, "hello", 0, 500));
        t.append(TranscriptSegment::new(Speaker::Others, "hi", 500, 900));
        let plain = t.plain_text();
        assert!(plain.contains("Me: hello"));
        assert!(plain.contains("Others: hi"));
    }

    #[test]
    fn format_ts_works() {
        assert_eq!(format_ts(0), "[00:00]");
        assert_eq!(format_ts(65_000), "[01:05]");
    }
}
