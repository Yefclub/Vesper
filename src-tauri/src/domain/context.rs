//! Notes the user types while a meeting is happening.
//!
//! Speech-to-text hears what was said; it does not hear how a client's name is
//! spelled, which decision was the real one, or that the last two minutes were
//! off the record. The person in the room knows all three at the time, and this
//! is where they say so.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextNote {
    pub id: i64,
    pub text: String,
    /// Offset from the start of the recording, in milliseconds.
    ///
    /// `None` for a note written after the recording stopped: there is no
    /// position for it to hold against the transcript, and inventing one would
    /// put it somewhere it never was.
    pub at_ms: Option<i64>,
    pub created_at: String,
}

/// Render notes for a prompt, or nothing at all.
///
/// Empty in, empty out — and the caller appends unconditionally, so this is
/// what keeps a meeting with no notes from carrying an empty heading that the
/// model would then feel obliged to fill.
pub fn notes_block(notes: &[ContextNote]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\nNotes the participant typed during the meeting. They are authoritative \
         about names, decisions and corrections — prefer them over the transcript \
         where the two disagree, and never contradict them:\n",
    );
    for note in notes {
        match note.at_ms {
            Some(ms) => out.push_str(&format!("- [{}] {}\n", stamp(ms), note.text)),
            None => out.push_str(&format!("- {}\n", note.text)),
        }
    }
    out
}

/// `mm:ss`, or `h:mm:ss` once there is an hour to show.
fn stamp(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Everything a summary is written from.
///
/// Grouped because the list stopped fitting: transcript, template, language and
/// now the participant's notes travel together to every provider, and passing
/// them one by one had each signature at eight parameters. They are one thing —
/// the meeting, as the model should see it.
pub struct SummarySubject<'a> {
    pub transcript: &'a str,
    pub template: crate::domain::summary::SummaryTemplate,
    pub locale: crate::domain::i18n::Locale,
    pub notes: &'a [ContextNote],
}

/// Everything a chat answer is grounded in, for the same reason.
pub struct ChatSubject<'a> {
    pub meeting_title: &'a str,
    pub transcript: &'a str,
    pub summary: Option<&'a str>,
    pub notes: &'a [ContextNote],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str, at_ms: Option<i64>) -> ContextNote {
        ContextNote {
            id: 1,
            text: text.into(),
            at_ms,
            created_at: "2026-08-04T00:00:00Z".into(),
        }
    }

    #[test]
    fn no_notes_add_nothing_to_the_prompt() {
        assert!(notes_block(&[]).is_empty());
    }

    #[test]
    fn a_note_keeps_its_place_in_the_recording() {
        let block = notes_block(&[note("Cliente = Acme", Some(65_000))]);
        assert!(block.contains("- [1:05] Cliente = Acme"), "{block}");
    }

    #[test]
    fn past_an_hour_the_stamp_grows_a_field() {
        let block = notes_block(&[note("Decisão", Some(3_725_000))]);
        assert!(block.contains("[1:02:05]"), "{block}");
    }

    /// A note written after the recording stopped has no offset, and must not
    /// be given one — a fabricated timestamp reads as evidence.
    #[test]
    fn a_note_without_a_position_shows_none() {
        let block = notes_block(&[note("Depois da reunião", None)]);
        assert!(block.contains("- Depois da reunião"), "{block}");
        assert!(!block.contains('['), "{block}");
    }
}
