use crate::domain::speaker::{Speaker, SpeakerNames};
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

    /// Replace one segment's words, keeping its clock.
    ///
    /// Local speech-to-text mishears names, acronyms and one-word answers, and
    /// the fastest route to notes worth trusting is a person fixing the line.
    /// The timing is not theirs to change — it came from the audio, and the
    /// summary, the search index and any future alignment all read it.
    ///
    /// Returns whether a segment by that id was there to edit. `false` is not a
    /// failure worth an error type: the id came from a list the caller was
    /// looking at, and the honest answer to "edit the row that is gone" is that
    /// nothing changed.
    pub fn edit_segment(&mut self, segment_id: &str, text: &str) -> bool {
        match self.segments.iter_mut().find(|s| s.id == segment_id) {
            Some(segment) => {
                segment.text = text.to_string();
                true
            }
            None => false,
        }
    }

    /// The lines as the meeting names its two channels.
    ///
    /// The names are the caller's to supply rather than the segment's own: they
    /// belong to the meeting, and the one thing that must not happen is a
    /// transcript on screen reading "Ana" while the copy the summary model was
    /// handed still reads "Me".
    pub fn plain_text(&self, names: &SpeakerNames) -> String {
        self.segments
            .iter()
            .map(|s| format!("{}: {}", names.label(s.speaker), s.text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The same lines, keeping the clock.
    ///
    /// `plain_text` drops the timestamps, which is right for a first summary —
    /// they are noise when the task is "what happened". A refinement is being
    /// asked to find what the first pass missed, and when something was said is
    /// most of how a model tells a decision from an aside.
    pub fn timestamped_text(&self, names: &SpeakerNames) -> String {
        self.segments
            .iter()
            .map(|s| {
                let secs = s.start_ms / 1000;
                format!(
                    "[{:02}:{:02}] {}: {}",
                    secs / 60,
                    secs % 60,
                    names.label(s.speaker),
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
    use crate::domain::i18n::Locale;

    fn english() -> SpeakerNames {
        SpeakerNames::resolve(None, None, Locale::En)
    }

    fn two_lines() -> LiveTranscript {
        let mut t = LiveTranscript::new();
        t.segments.push(TranscriptSegment {
            id: "a".into(),
            speaker: Speaker::Me,
            text: "clode Cote".into(),
            start_ms: 0,
            end_ms: 1_000,
        });
        t.segments.push(TranscriptSegment {
            id: "b".into(),
            speaker: Speaker::Others,
            text: "sim".into(),
            start_ms: 1_000,
            end_ms: 2_000,
        });
        t
    }

    #[test]
    fn an_edit_changes_the_words_and_nothing_else() {
        let mut t = two_lines();
        assert!(t.edit_segment("a", "Claude Code"));
        assert_eq!(t.segments[0].text, "Claude Code");
        assert_eq!(t.segments[0].start_ms, 0, "the clock is not the user's");
        assert_eq!(t.segments[0].end_ms, 1_000);
        assert_eq!(t.segments[0].speaker, Speaker::Me);
        assert_eq!(t.segments[1].text, "sim", "only the named line moves");
    }

    #[test]
    fn editing_a_line_that_is_gone_changes_nothing() {
        let mut t = two_lines();
        assert!(!t.edit_segment("zzz", "x"));
        assert_eq!(t.segments.len(), 2);
    }

    /// The summary reads `plain_text`, so an edit has to reach it or the
    /// correction would be visible on screen and invisible to the model.
    #[test]
    fn the_corrected_words_are_what_the_summary_will_read() {
        let mut t = two_lines();
        t.edit_segment("a", "Claude Code");
        assert!(t.plain_text(&english()).contains("Claude Code"));
        assert!(!t.plain_text(&english()).contains("clode Cote"));
    }

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
        let plain = t.plain_text(&english());
        assert!(plain.contains("Me: hello"));
        assert!(plain.contains("Others: hi"));
    }

    /// The text every prompt is built from. A name that reached the screen and
    /// not this would leave the model writing about "Others" in a summary the
    /// reader sees headed "Cliente".
    #[test]
    fn the_meetings_own_names_are_what_the_model_reads() {
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(Speaker::Me, "hello", 0, 500));
        t.append(TranscriptSegment::new(Speaker::Others, "hi", 500, 900));
        let names = SpeakerNames::resolve(Some("Ana"), Some("Cliente"), Locale::En);
        assert_eq!(t.plain_text(&names), "Ana: hello\nCliente: hi");
        assert_eq!(
            t.timestamped_text(&names),
            "[00:00] Ana: hello\n[00:00] Cliente: hi"
        );
    }

    #[test]
    fn format_ts_works() {
        assert_eq!(format_ts(0), "[00:00]");
        assert_eq!(format_ts(65_000), "[01:05]");
    }
}
