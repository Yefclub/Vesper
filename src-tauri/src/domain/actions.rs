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

    let mut kept: Vec<ActionItem> = Vec::new();
    for item in existing {
        // A suggestion the model makes again is the same piece of work, so the
        // row — and the status on it — stays.
        let repeated = suggested.iter().any(|s| same_work(s, &item.text));
        if protected(item) || repeated {
            kept.push(item.clone());
        }
    }

    for text in suggested {
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
        }
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
