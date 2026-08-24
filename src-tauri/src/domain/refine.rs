//! Improving one section of a summary without losing the one before it.

use crate::domain::i18n::Locale;
use serde::{Deserialize, Serialize};

/// Which part of the summary an improvement is aimed at.
///
/// A typed pair rather than a free string: this arrives from the WebView and
/// chooses a prompt and a merge target, so an unrecognised value has to be a
/// rejection at the boundary rather than a silent no-op deeper in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    KeyPoints,
    ActionItems,
}

impl Section {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "key_points" => Some(Self::KeyPoints),
            "action_items" => Some(Self::ActionItems),
            _ => None,
        }
    }

    /// The wire name, which is also what a stored version records as its origin.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KeyPoints => "key_points",
            Self::ActionItems => "action_items",
        }
    }
}

/// Where a stored version came from.
///
/// Kept as a string on the wire and in the row so a version written by an older
/// build reads back rather than failing the whole history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryVersion {
    pub version: i64,
    pub origin: String,
    pub created_at: String,
    pub summary: String,
    pub key_points: String,
    pub action_items: String,
    /// The whole section set as it stood, JSON. A version is the WHOLE insight
    /// set — restoring one that carried only the three fields above would erase
    /// a client call's Requirements and Risks, which no field here holds.
    #[serde(default)]
    pub sections_json: Option<String>,
}

/// Ask the model to improve one section, given everything the first pass had
/// plus what it already answered.
///
/// The first pass already sends the whole transcript, so there was no truncation
/// to widen — "collecting more information" is met by giving the model the
/// *timestamped, speaker-labelled* transcript instead of the flattened text, and
/// by showing it its own previous answer so it can extend rather than restate.
///
/// The instruction names the one section on purpose. A prompt that says "improve
/// the summary" comes back having rewritten all three, and the user asked for
/// this section.
pub fn build_refine_prompt(
    section: Section,
    current: &str,
    transcript: &str,
    locale: Locale,
) -> String {
    let (label, aim) = match section {
        Section::KeyPoints => (
            "key points",
            "Add what the current list is missing, merge duplicates, and cut anything the transcript does not support.",
        ),
        Section::ActionItems => (
            "action items",
            "Add actions the transcript states or clearly implies, name the owner where the transcript names one, and cut anything nobody committed to.",
        ),
    };
    // Action items are the one section stored as rows rather than as text, and
    // each row carries the moment it was decided. Without this the summary's
    // items are playable and anything this path adds is not — the same list,
    // half of it citable, which reads as the citation being unreliable rather
    // than absent. The transcript here is already the timestamped one.
    // No indentation on the inserted line. The rules above are written inside a
    // line-continued literal, where the leading whitespace of each source line
    // is stripped; this one is not, so spaces put here for tidiness would reach
    // the model as an indented block rather than a rule beside the others.
    let cite = match section {
        Section::ActionItems => {
            "\n- end every line with the moment it was decided, in square brackets, \
             copied from the timestamp of the transcript line it came from: `[mm:ss]`. \
             Put it last, after any owner or deadline, and leave it off rather than guess one"
        }
        Section::KeyPoints => "",
    };
    let language = match locale {
        Locale::PtBr => "Write the list in Brazilian Portuguese.",
        Locale::En => "Write the list in English.",
    };
    format!(
        "Improve the {label} for this meeting.\n\n\
         Rules:\n\
         - answer with the improved {label} only, one per line, each starting with `- `\n\
         - no heading, no preamble, no closing remark\n\
         - {aim}\n\
         - {language}{cite}\n\
         - if the transcript genuinely supports nothing, answer with the current list unchanged\n\n\
         Current {label}:\n{current}\n\n\
         Transcript:\n{transcript}\n"
    )
}

/// Pull a bullet list out of whatever the model answered.
///
/// Models bullet with `-`, `*`, `•` or a number however firmly they are asked
/// not to, and some open with a sentence regardless. Lines that survive are the
/// ones shaped like list items; if none are, the answer is not a list and the
/// caller keeps what it had.
pub fn parse_refined_list(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let t = line.trim();
            let t = t
                .strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))
                .or_else(|| t.strip_prefix("• "))
                .or_else(|| numbered(t))?;
            let t = t.trim();
            (!t.is_empty()).then(|| t.to_string())
        })
        .collect()
}

/// `3. Ship it` → `Ship it`.
fn numbered(t: &str) -> Option<&str> {
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let rest = &t[digits..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_section_is_rejected_at_the_boundary() {
        assert_eq!(Section::parse("key_points"), Some(Section::KeyPoints));
        assert_eq!(Section::parse("action_items"), Some(Section::ActionItems));
        assert_eq!(Section::parse("summary"), None);
        assert_eq!(Section::parse(""), None);
    }

    /// The prompt has to name one section, or the model rewrites all three and
    /// the user loses the two they did not ask about.
    #[test]
    fn the_prompt_names_only_the_section_asked_for() {
        let p = build_refine_prompt(
            Section::ActionItems,
            "- do a thing",
            "0:00 Me: hi",
            Locale::En,
        );
        assert!(p.contains("action items"));
        assert!(!p.contains("key points"));
        assert!(p.contains("- do a thing"));
        assert!(p.contains("0:00 Me: hi"));
    }

    /// Action items are rows carrying the moment they were decided, and a
    /// refinement adds to that list. Without the instruction the summary's items
    /// would be playable and anything added here would not — the same list, half
    /// of it citable, which reads as the citation being unreliable.
    #[test]
    fn refining_action_items_asks_for_the_moment_too() {
        let p = build_refine_prompt(Section::ActionItems, "", "0:00 Me: hi", Locale::En);
        assert!(p.contains("[mm:ss]"), "{p}");
        assert!(p.contains("rather than guess one"), "{p}");
        // Beside the other rules, not indented under one. The rules above are
        // written inside a line-continued literal and arrive at column zero;
        // whitespace put on this one for tidiness would survive and make it an
        // indented block the model reads as an example rather than an
        // instruction.
        let rule = p
            .lines()
            .find(|l| l.contains("[mm:ss]"))
            .expect("the rule is on a line of its own");
        assert!(rule.starts_with("- end every line"), "{rule:?}");
    }

    /// Key points are stored as text and nothing plays them back. Asking for a
    /// stamp there would put brackets in the middle of a summary.
    #[test]
    fn refining_key_points_does_not() {
        let p = build_refine_prompt(Section::KeyPoints, "", "0:00 Me: hi", Locale::En);
        assert!(!p.contains("[mm:ss]"), "{p}");
    }

    #[test]
    fn the_list_is_written_in_the_users_language() {
        let p = build_refine_prompt(Section::KeyPoints, "", "", Locale::PtBr);
        assert!(p.contains("Brazilian Portuguese"));
    }

    #[test]
    fn every_shape_of_bullet_a_model_answers_with_is_read() {
        let raw = "Here is the improved list:\n- first\n* second\n• third\n4. fourth\n5) fifth\n\n";
        assert_eq!(
            parse_refined_list(raw),
            vec!["first", "second", "third", "fourth", "fifth"]
        );
    }

    /// A model that answered with prose has not produced a list, and the caller
    /// has to be able to tell so it can keep what it had.
    #[test]
    fn prose_yields_nothing_rather_than_a_single_bad_item() {
        assert!(parse_refined_list("I could not improve this list.").is_empty());
        assert!(parse_refined_list("").is_empty());
    }
}
