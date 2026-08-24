//! Action items as work, not as a paragraph.
//!
//! The value of a meeting is what happens next, and a free-text bullet list
//! cannot be ticked off, cannot carry an owner, and is replaced wholesale every
//! time the meeting is summarised again. Anything the user corrected is gone
//! with it, which teaches people not to correct it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    #[default]
    Open,
    Done,
}

/// Who put the item there.
///
/// The distinction exists for one rule: a re-summarise may replace what the
/// model said last time, and may never replace what a person said or touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionSource {
    #[default]
    Ai,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItem {
    pub id: i64,
    pub text: String,
    pub owner: Option<String>,
    /// Free text rather than a date type, on purpose: what a meeting produces is
    /// "Friday", "end of the month", "before the demo". Parsing that into a
    /// timestamp would be inventing a precision nobody agreed to.
    pub due: Option<String>,
    pub status: ActionStatus,
    pub source: ActionSource,
    /// Whether a person has changed this since the model wrote it. Set on any
    /// edit, and never cleared — it is what protects the item from the next
    /// summary.
    pub edited: bool,
    /// Where in the recording this was decided, in milliseconds from the start.
    ///
    /// The model is asked to end each suggestion with the moment it came up,
    /// and the offset is split off the text and kept here. It is what turns an
    /// action item from something to take on trust into something you can play
    /// back — and an item the model invented has no moment to point at, so a
    /// missing or wrong one is visible rather than plausible.
    ///
    /// `None` for anything a person wrote, for a model that ignored the
    /// instruction, and for every item that predates this column.
    #[serde(default)]
    pub at_ms: Option<i64>,
}

/// Split a trailing `[m:ss]`, `[mm:ss]` or `[h:mm:ss]` off a suggestion.
///
/// Returns the text without it and the offset in milliseconds. Anything that is
/// not a stamp is left where it is: a line ending in `[TBD]` is text, and
/// eating it would silently shorten somebody's task.
pub fn split_stamp(line: &str) -> (String, Option<i64>) {
    let bare = || (line.trim().to_string(), None);
    let trimmed = line.trim_end();
    if !trimmed.ends_with(']') {
        return bare();
    }
    let Some(open) = trimmed.rfind('[') else {
        return bare();
    };
    match parse_stamp(&trimmed[open + 1..trimmed.len() - 1]) {
        Some(ms) => (trimmed[..open].trim().to_string(), Some(ms)),
        None => bare(),
    }
}

/// `m:ss` or `h:mm:ss` as milliseconds.
///
/// The minutes are **not** bounded at 60 in the two-field form, and that is not
/// leniency — it is the vocabulary the transcript itself uses.
/// `timestamped_text` writes `[{minutes:02}:{seconds:02}]` from the total, so an
/// hour and a quarter into a recording it emits `[75:00]`, and the prompt asks
/// the model to copy what it was given. Refusing that would leave every action
/// decided after the first hour without the citation this whole change is for.
///
/// Seconds are bounded, and so are the minutes of a three-field stamp: those
/// are out of range in any reading, and a stamp that cannot be read is better
/// left in the text than turned into a seek to a moment that was never there.
fn parse_stamp(s: &str) -> Option<i64> {
    let parts: Vec<i64> = s
        .split(':')
        .map(|p| p.trim().parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let (h, m, sec) = match parts[..] {
        [m, sec] => (0, m, sec),
        [h, m, sec] if (0..60).contains(&m) => (h, m, sec),
        _ => return None,
    };
    if h < 0 || m < 0 || !(0..60).contains(&sec) {
        return None;
    }
    Some((h * 3600 + m * 60 + sec) * 1000)
}

/// What a re-summarise is allowed to do to the existing list.
///
/// The rule in one sentence: **the model may replace its own untouched
/// suggestions and nothing else.**
///
/// - An item the user wrote survives.
/// - An item the user edited or ticked survives, with their words, not the
///   model's.
/// - An untouched suggestion the model no longer makes disappears, because it
///   was never anything but a suggestion.
/// - A suggestion the model repeats keeps its existing row, so a status set on
///   it is not lost to a re-run that said the same thing.
///
/// Returns the items to keep, in a stable order: everything that survived,
/// then the suggestions that are new this time.
pub fn merge_suggestions(existing: &[ActionItem], suggested: &[String]) -> Vec<ActionItem> {
    let protected = |i: &ActionItem| {
        i.source == ActionSource::User || i.edited || i.status == ActionStatus::Done
    };

    // The stamp comes off before anything is compared. A model asked twice puts
    // the same task a second either side of where it put it last time, and
    // comparing the stamped lines would make that a different piece of work —
    // the list would grow by its whole length on every re-summarise.
    let suggested: Vec<(String, Option<i64>)> = suggested.iter().map(|s| split_stamp(s)).collect();

    let mut kept: Vec<ActionItem> = Vec::new();
    for item in existing {
        // A suggestion the model makes again is the same piece of work, so the
        // row — and the status on it — stays.
        let repeated = suggested.iter().find(|(s, _)| same_work(s, &item.text));
        match repeated {
            // Its own untouched suggestion is the one thing the model is allowed
            // to replace, so a stamp it did not produce last time can land now.
            // A protected row keeps everything it had, including no stamp.
            Some((_, at_ms)) if !protected(item) => kept.push(ActionItem {
                at_ms: *at_ms,
                ..item.clone()
            }),
            Some(_) => kept.push(item.clone()),
            None if protected(item) => kept.push(item.clone()),
            None => {}
        }
    }

    for (text, at_ms) in &suggested {
        if kept.iter().any(|i| same_work(text, &i.text)) {
            continue;
        }
        kept.push(ActionItem {
            id: 0,
            text: text.clone(),
            owner: None,
            due: None,
            status: ActionStatus::Open,
            source: ActionSource::Ai,
            edited: false,
            at_ms: *at_ms,
        });
    }
    kept
}

/// Whether two lines describe the same piece of work.
///
/// Compared without case, surrounding space or a trailing full stop: a model
/// asked the same question twice writes the same sentence with different
/// punctuation about as often as not, and treating those as two tasks would
/// grow the list every time a meeting is re-summarised.
fn same_work(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_end_matches('.')
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ai(id: i64, text: &str) -> ActionItem {
        ActionItem {
            id,
            text: text.into(),
            owner: None,
            due: None,
            status: ActionStatus::Open,
            source: ActionSource::Ai,
            edited: false,
            at_ms: None,
        }
    }

    #[test]
    fn a_stamp_comes_off_the_end_and_becomes_an_offset() {
        assert_eq!(
            split_stamp("Assinar o build [12:05]"),
            ("Assinar o build".to_string(), Some(725_000))
        );
        assert_eq!(
            split_stamp("Assinar o build [1:02:05]"),
            ("Assinar o build".to_string(), Some(3_725_000))
        );
    }

    /// A line that merely ends in brackets is a line, not a citation.
    #[test]
    fn something_that_is_not_a_stamp_stays_in_the_text() {
        for line in [
            "Definir o prazo [TBD]",
            "Rever o item [3]",
            "Conferir a nota [a:b]",
            "Sem colchete nenhum",
        ] {
            assert_eq!(split_stamp(line), (line.trim().to_string(), None), "{line}");
        }
    }

    /// The transcript the model is given writes minutes from the total, so an
    /// hour and a quarter in it says `[75:00]` — and the prompt asks the model
    /// to copy what it was given. Refusing that would leave every item decided
    /// after the first hour without a citation, which is most of a long meeting.
    #[test]
    fn minutes_past_the_hour_are_the_transcripts_own_vocabulary() {
        assert_eq!(
            split_stamp("Publicar [75:00]"),
            ("Publicar".to_string(), Some(4_500_000))
        );
    }

    /// Out of range in any reading. A stamp that cannot be read is better left
    /// in the text than turned into a seek to a moment that was never there.
    #[test]
    fn an_unreadable_stamp_is_refused_rather_than_normalised() {
        for line in [
            "Publicar [10:61]",
            "Publicar [1:75:00]",
            "Publicar [1:2:3:4]",
        ] {
            assert_eq!(split_stamp(line), (line.to_string(), None), "{line}");
        }
    }

    /// The stamp must not make the same task look new. A model asked twice puts
    /// it a second either side of where it was, and the list would double.
    #[test]
    fn a_moved_stamp_is_still_the_same_piece_of_work() {
        let existing = vec![ActionItem {
            at_ms: Some(725_000),
            ..ai(1, "Assinar o build")
        }];
        let kept = merge_suggestions(&existing, &["Assinar o build [12:07]".into()]);
        assert_eq!(kept.len(), 1, "one task, not two");
        assert_eq!(kept[0].id, 1, "and it kept its row");
        assert_eq!(kept[0].at_ms, Some(727_000), "with the newer moment");
    }

    /// The stamp is the model's, so it may refresh its own suggestion — and may
    /// not touch a row a person has claimed.
    #[test]
    fn a_stamp_never_lands_on_an_item_a_person_touched() {
        let existing = vec![ActionItem {
            edited: true,
            ..ai(1, "Assinar o build")
        }];
        let kept = merge_suggestions(&existing, &["Assinar o build [12:07]".into()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].at_ms, None, "their row, their contents");
    }

    #[test]
    fn a_new_suggestion_arrives_with_its_moment() {
        let kept = merge_suggestions(&[], &["Publicar a release [0:45]".into()]);
        assert_eq!(kept[0].text, "Publicar a release");
        assert_eq!(kept[0].at_ms, Some(45_000));
    }

    #[test]
    fn an_untouched_suggestion_the_model_dropped_goes_away() {
        let kept = merge_suggestions(&[ai(1, "Assinar o build")], &["Publicar a release".into()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "Publicar a release");
        assert_eq!(kept[0].id, 0, "a new suggestion has no row yet");
    }

    #[test]
    fn what_the_user_wrote_survives_a_re_summarise() {
        let mine = ActionItem {
            source: ActionSource::User,
            ..ai(7, "Ligar para o cliente")
        };
        let kept = merge_suggestions(std::slice::from_ref(&mine), &["Outra coisa".into()]);
        assert!(kept.contains(&mine), "{kept:?}");
    }

    #[test]
    fn an_edited_item_keeps_the_users_words() {
        let edited = ActionItem {
            edited: true,
            text: "Assinar o build ANTES da demo".into(),
            ..ai(3, "")
        };
        let kept = merge_suggestions(std::slice::from_ref(&edited), &["Assinar o build".into()]);
        assert!(kept.contains(&edited));
        assert_eq!(kept.len(), 2, "the suggestion is not the same work");
    }

    /// Ticking something off is a statement about the world. A re-run that no
    /// longer mentions it has not undone it.
    #[test]
    fn a_finished_item_is_not_removed_by_a_re_run() {
        let done = ActionItem {
            status: ActionStatus::Done,
            ..ai(4, "Enviar a ata")
        };
        let kept = merge_suggestions(std::slice::from_ref(&done), &[]);
        assert_eq!(kept, vec![done]);
    }

    /// The same suggestion, said again, must not become a second task — and the
    /// status somebody set on it must not be reset.
    #[test]
    fn a_repeated_suggestion_keeps_its_row_and_its_status() {
        let existing = ActionItem {
            status: ActionStatus::Done,
            ..ai(9, "Assinar o build")
        };
        let kept = merge_suggestions(&[existing], &["  assinar o build.  ".into()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, 9, "the existing row is kept");
        assert_eq!(kept[0].status, ActionStatus::Done);
    }

    #[test]
    fn the_first_summary_of_a_meeting_is_just_the_suggestions() {
        let kept = merge_suggestions(&[], &["A".into(), "B".into()]);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|i| i.source == ActionSource::Ai));
    }
}
