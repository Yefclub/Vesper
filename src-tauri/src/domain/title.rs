//! Generated meeting titles: the prompt, the parser, and the date label the
//! generator is allowed to replace.
//!
//! The date label is the whole safety mechanism. A meeting keeps the title it
//! has unless that title is still exactly the shape `fallback_title` writes, so
//! a name the user typed and an import's file stem are both out of reach —
//! without a `titled_by_user` column, i.e. without altering the only copy
//! anyone has of their meetings.

use chrono::{DateTime, Local, NaiveDateTime};

/// Prefix and time format of the date label, shared by the writer and the
/// recogniser. Two literals would drift and the guard would fail *open*, which
/// hands the generator every title in the database.
const FALLBACK_PREFIX: &str = "Meeting ";
const FALLBACK_FORMAT: &str = "%Y-%m-%d %H:%M";

/// Longest title kept. Chars, never bytes: a byte cut lands inside the accented
/// half of the languages this app is used in.
const MAX_TITLE_CHARS: usize = 72;

/// How much transcript goes into the prompt beside the summary.
const EXCERPT_CHARS: usize = 400;

/// Labels a model writes even after being told not to.
const TITLE_LABELS: [&str; 3] = ["title", "título", "titulo"];

/// The placeholder a meeting is born with.
///
/// Local time, deliberately, while `created_at` is UTC: a title is a *label*
/// frozen at creation. Fly to Lisbon and it is still the meeting you had at
/// 22:33 in São Paulo. The sortable timestamp is the one that must not move.
pub fn fallback_title(now: DateTime<Local>) -> String {
    format!("{FALLBACK_PREFIX}{}", now.format(FALLBACK_FORMAT))
}

/// Whether this title is still the date label, and may therefore be replaced.
///
/// Shape, not equality against `now`: the label is frozen at creation and the
/// generator runs minutes later, in a session that may have crossed a timezone.
pub fn is_fallback_title(title: &str) -> bool {
    title
        .strip_prefix(FALLBACK_PREFIX)
        .map(|rest| NaiveDateTime::parse_from_str(rest, FALLBACK_FORMAT).is_ok())
        .unwrap_or(false)
}

/// Ask for a title from the summary *and* the meeting's own words.
///
/// Both inputs earn their place: the summary carries the subject, and the
/// excerpt carries the language and register. A summary written in English from
/// a Portuguese meeting would otherwise title it in English.
///
/// The excerpt is cut by chars — a byte slice of a transcript panics the moment
/// the cut lands inside an accented character.
pub fn build_title_prompt(
    summary: &str,
    transcript: &str,
    locale: crate::domain::i18n::Locale,
) -> String {
    let excerpt: String = transcript.trim().chars().take(EXCERPT_CHARS).collect();
    format!(
        "Name this meeting.\n\nRules:\n\
         - at most 6 words\n\
         - answer with the title alone: no quotes, no `Title:` prefix, no trailing punctuation, no explanation\n\
         - write it in {}\n\n\
         Summary:\n{}\n\nTranscript excerpt:\n{}\n\n{}",
        crate::domain::summary::language_name(locale),
        summary.trim(),
        excerpt,
        crate::domain::summary::write_in_language(locale)
    )
}

/// Reduce whatever the model answered — or the user typed — to one title.
///
/// `None` means nothing printable survived, which is the caller's signal to
/// keep the date label rather than write an empty name.
pub fn parse_title(raw: &str) -> Option<String> {
    let line = raw.lines().find(|l| !l.trim().is_empty())?;

    // Whitespace first, control chars second: a tab is both, and dropping it as
    // a control char before it counts as a gap would join the words around it.
    let mut cleaned = String::with_capacity(line.len());
    let mut gap = false;
    for ch in line.chars() {
        if ch.is_whitespace() {
            gap = !cleaned.is_empty();
            continue;
        }
        if ch.is_control() {
            continue;
        }
        if gap {
            cleaned.push(' ');
            gap = false;
        }
        cleaned.push(ch);
    }

    // Peel until nothing changes: `**"Title: Weekly sync"**` needs several
    // passes and arrives in no fixed order.
    let mut t: &str = &cleaned;
    loop {
        let before = t;
        t = t.trim();
        t = t.trim_matches(|c| {
            matches!(
                c,
                '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}' | '*'
            )
        });
        if let Some((head, rest)) = t.split_once(':') {
            // "Client call: Q3 budget" keeps its colon — only the label goes.
            if TITLE_LABELS.contains(&head.trim().to_lowercase().as_str()) {
                t = rest;
            }
        }
        if t == before {
            break;
        }
    }

    let cut: String = t.chars().take(MAX_TITLE_CHARS).collect();
    let cut = cut.trim_end();
    // A title has to say something outside a marker. Whisper writes `[BLANK_AUDIO]`
    // for silence and `(downbeat music)`, `(music)`, `(applause)` for anything it
    // heard but could not transcribe, and a model handed nothing else answers
    // with them — two recordings came out named `: [BLANK_AUDIO]?` and
    // `: (downbeat music)`.
    //
    // Rejecting rather than stripping: a model that had only markers to work with
    // has not named anything, and the date label is the honest fallback. A title
    // that merely *contains* one — "Weekly sync (Q3)" — still has words of its
    // own outside it and passes.
    (!cut.is_empty() && names_something(cut)).then(|| cut.to_string())
}

/// Whether anything outside a `[...]` or `(...)` span is alphanumeric.
fn names_something(title: &str) -> bool {
    let mut depth = 0usize;
    for ch in title.chars() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            c if depth == 0 && c.is_alphanumeric() => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Whisper's silence marker is not a name. A recording of an empty room came
    /// back titled `: [BLANK_AUDIO]?` before this.
    #[test]
    fn a_transcript_of_silence_does_not_produce_a_title() {
        assert_eq!(parse_title(": [BLANK_AUDIO]?"), None);
        assert_eq!(parse_title("[BLANK_AUDIO]"), None);
        assert_eq!(parse_title("[ Silence ]"), None);
        // Whisper's other shape, for audio it heard but could not transcribe.
        assert_eq!(parse_title(": (downbeat music)"), None);
        assert_eq!(parse_title("(music)"), None);
        assert_eq!(parse_title("(applause) [BLANK_AUDIO]"), None);
        assert_eq!(parse_title("**\"[BLANK_AUDIO]\"**"), None);
        // A real title keeps working, including one that happens to bracket
        // something inside it.
        assert_eq!(
            parse_title("Weekly sync [Q3]").as_deref(),
            Some("Weekly sync [Q3]")
        );
        assert_eq!(
            parse_title("Weekly sync (Q3)").as_deref(),
            Some("Weekly sync (Q3)")
        );
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    /// The round trip is the guard. Writer and recogniser share one prefix and
    /// one format string; if they ever stop agreeing, every user-typed title
    /// becomes fair game for the generator and this is what catches it.
    #[test]
    fn a_written_fallback_title_is_recognised_as_one() {
        for now in [
            at(2026, 8, 1, 22, 33),
            at(2026, 1, 1, 0, 0),
            at(2026, 12, 31, 23, 59),
        ] {
            assert!(is_fallback_title(&fallback_title(now)));
        }
    }

    #[test]
    fn a_renamed_title_is_never_overwritten() {
        // What the user typed.
        assert!(!is_fallback_title("Sprint planning"));
        // Starts with the prefix and still is not the label.
        assert!(!is_fallback_title("Meeting notes"));
        assert!(!is_fallback_title("Meeting 2026-08-01"));
        assert!(!is_fallback_title("Meeting with Ana 2026-08-01 22:33"));
        // An import's file stem.
        assert!(!is_fallback_title("gravacao-2026-08-01"));
        assert!(!is_fallback_title(""));
    }

    /// The hole this predicate cannot close on its own, and the reason a meeting
    /// carries `title_locked`: what the user typed is allowed to look exactly
    /// like the label, and shape alone would hand it back to the generator.
    #[test]
    fn a_user_title_shaped_like_the_label_is_still_theirs() {
        let typed = fallback_title(Local::now());
        assert!(
            is_fallback_title(&typed),
            "shape cannot tell these apart — provenance has to"
        );
    }

    #[test]
    fn a_model_that_answers_with_a_paragraph_yields_one_line() {
        let raw = "**Title: Weekly roadmap sync**\n\nI chose this because the \
                   speakers spend most of the call on the roadmap.";
        assert_eq!(parse_title(raw).unwrap(), "Weekly roadmap sync");
        assert_eq!(
            parse_title("\"Alinhamento de produto\"").unwrap(),
            "Alinhamento de produto"
        );
        // A colon that belongs to the title survives.
        assert_eq!(
            parse_title("Client call: Q3 budget").unwrap(),
            "Client call: Q3 budget"
        );
    }

    #[test]
    fn a_title_cut_at_72_chars_stays_valid_utf8() {
        let long = "ação ".repeat(40);
        let cut = parse_title(&long).unwrap();
        assert!(cut.chars().count() <= MAX_TITLE_CHARS);
        // The point of the test: 72 chars of this is more than 72 bytes, so a
        // byte cut would have split a character and panicked on the way in.
        assert!(cut.len() > MAX_TITLE_CHARS);
        assert!(cut.contains("ação"));
    }

    #[test]
    fn a_model_that_says_nothing_useful_yields_none() {
        assert_eq!(parse_title(""), None);
        assert_eq!(parse_title("\n \n\t\n"), None);
        assert_eq!(parse_title("\"\""), None);
        assert_eq!(parse_title("**"), None);
        assert_eq!(parse_title("Title:"), None);
    }

    #[test]
    fn the_prompt_carries_both_the_summary_and_the_meetings_own_words() {
        let transcript = "Me: então vamos falar da migração. ".repeat(40);
        let p = build_title_prompt(
            "Migration planning",
            &transcript,
            crate::domain::i18n::Locale::En,
        );
        assert!(p.contains("Migration planning"));
        assert!(p.contains("migração"));
        assert!(p.contains("at most 6 words"));
        // Excerpt, not the whole transcript, and cut on a char boundary.
        assert!(p.matches("migração").count() < transcript.matches("migração").count());
    }

    /// The title follows the user's setting, not the model's reading of the
    /// transcript. Guessing is what named a Portuguese meeting in English: the
    /// speech was full of product names that are English nouns, and the model
    /// went with those over the words around them.
    #[test]
    fn the_title_prompt_names_the_configured_language() {
        use crate::domain::i18n::Locale;
        let pt = build_title_prompt("Resumo", "Eu: bom dia.", Locale::PtBr);
        assert!(pt.contains("Brazilian Portuguese"));
        assert!(pt.contains("Escreva tudo em português do Brasil."));
        assert!(
            !pt.contains("the language the meeting was held in"),
            "the prompt must not ask the model to guess"
        );
        let en = build_title_prompt("Resumo", "Eu: bom dia.", Locale::En);
        assert!(en.contains("English"));
        assert!(!en.contains("português"));
    }
}
