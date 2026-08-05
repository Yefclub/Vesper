//! The words a model keeps getting wrong, and how they reach it.
//!
//! whisper has no vocabulary API. `whisper-rs` 0.16 exposes `set_initial_prompt`
//! and raw prompt tokens, and nothing that pins a word — so a hot-word list can
//! only be text the decoder is told came immediately before the audio. That
//! makes a term *more likely*, never certain: "Yef" in the list biases the
//! decoder towards that spelling, and a model that cannot hear the difference
//! will still write "Yeff". An engine with no prompt of any kind ignores the
//! list entirely and transcribes exactly as it does today; nothing here fails,
//! the words simply keep coming back wrong.
//!
//! This is also the one place text a user typed reaches a model prompt, and the
//! shortest path from here to whisper is `FullParams::set_initial_prompt`, which
//! builds a `CString` and **panics on a null byte**. Everything leaving this
//! module is stripped of control characters and bounded, so a pasted binary file
//! costs a useless prompt rather than every transcription of the meeting.

/// How many terms are carried. A vocabulary is a handful of names, not a
/// dictionary — past this the prompt is mostly words that were never spoken,
/// which is what pulls a transcript towards them.
const MAX_TERMS: usize = 64;

/// Longest single term. A "term" past this is a pasted paragraph.
const MAX_TERM_CHARS: usize = 64;

/// Character budget for the vocabulary half of a prompt.
///
/// whisper.cpp truncates the initial prompt to half its text context — 224
/// tokens — and the live pass appends up to `PROMPT_TAIL_CHARS` of the previous
/// utterance after this. Keeping the vocabulary here leaves the pair comfortably
/// inside that window, so neither half is silently cut: a truncated prompt would
/// drop terms with no way to tell which.
const MAX_VOCAB_CHARS: usize = 320;

/// The user's list, as it is worth storing: one term per entry, trimmed, unique,
/// bounded, and free of anything that cannot go into a C string.
///
/// Splitting on lines rather than commas is deliberate — the field is a textarea
/// and reads as one per line, and a company called "Acme, Inc." is one term.
pub fn normalise(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.iter().flat_map(|entry| entry.lines()) {
        // Controls dropped rather than replaced by a space: a stray tab inside a
        // term is a typo, and gluing the halves back together is closer to what
        // was meant than splitting one name into two.
        let cleaned: String = line.chars().filter(|c| !c.is_control()).collect();
        // Truncated by character, never by byte — this list is as often
        // Portuguese as English, and cutting an accented letter in half panics.
        let term: String = cleaned.trim().chars().take(MAX_TERM_CHARS).collect();
        if term.is_empty() || out.contains(&term) {
            continue;
        }
        out.push(term);
        if out.len() == MAX_TERMS {
            break;
        }
    }
    out
}

/// The prompt to hand the engine: the vocabulary, then whatever the speaker was
/// last heard saying.
///
/// No label in front of the terms. "Glossary:" would be English text at the head
/// of a Portuguese transcription's prompt, and whisper reads the prompt as
/// speech that just happened — a word in the wrong language there is a hint that
/// the meeting is in that language.
///
/// The continuation goes last because it is the sentence the audio continues
/// from, and the closer it sits to the audio the better it reads as one. With no
/// terms this returns the continuation unchanged, which is what every existing
/// caller was passing before a vocabulary existed.
pub fn initial_prompt(terms: &[String], continuation: &str) -> String {
    let mut vocab = String::new();
    for term in terms {
        let separator = if vocab.is_empty() { 0 } else { 2 };
        if vocab.chars().count() + separator + term.chars().count() > MAX_VOCAB_CHARS {
            // Whole terms only. A half-written name in the prompt is a
            // misspelling the decoder is being asked to reproduce.
            break;
        }
        if !vocab.is_empty() {
            vocab.push_str(", ");
        }
        vocab.push_str(term);
    }
    let continuation = continuation.trim();
    let prompt = match (vocab.is_empty(), continuation.is_empty()) {
        (true, _) => continuation.to_string(),
        (false, true) => format!("{vocab}."),
        (false, false) => format!("{vocab}. {continuation}"),
    };
    // Scrubbed again, for the continuation's sake: it is a stored transcript
    // line, and `edit_transcript_segment` lets the WebView write those. The
    // panic this avoids is in whisper-rs, one call further down.
    prompt.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn blank_lines_and_padding_are_dropped() {
        let got = normalise(&terms(&["  Vesper  ", "", "   ", "OpenRouter"]));
        assert_eq!(got, vec!["Vesper", "OpenRouter"]);
    }

    #[test]
    fn one_entry_holding_several_lines_becomes_several_terms() {
        // What a textarea sends: the whole field in one string.
        let got = normalise(&terms(&["Vesper\nYef\nwhisper.cpp"]));
        assert_eq!(got, vec!["Vesper", "Yef", "whisper.cpp"]);
    }

    #[test]
    fn the_same_term_twice_is_kept_once() {
        let got = normalise(&terms(&["Vesper", "Vesper"]));
        assert_eq!(got, vec!["Vesper"]);
    }

    /// The pathological value. `set_initial_prompt` builds a `CString` and
    /// panics on a null byte, so a list carrying one would fail every
    /// transcription of the meeting rather than merely being useless.
    #[test]
    fn control_characters_never_survive_normalisation() {
        let got = normalise(&terms(&["Ves\0per\u{7}", "\u{1b}[31mYef"]));
        assert_eq!(got, vec!["Vesper", "[31mYef"]);
        assert!(!initial_prompt(&got, "").chars().any(char::is_control));
    }

    /// And not through the continuation either, which comes from a stored
    /// transcript line the WebView is allowed to write.
    #[test]
    fn a_control_character_in_the_continuation_is_stripped_too() {
        let prompt = initial_prompt(&terms(&["Vesper"]), "he said\0 something");
        assert!(!prompt.chars().any(char::is_control));
        assert!(prompt.contains("he said something"));
    }

    #[test]
    fn a_term_longer_than_the_cap_is_cut_on_a_character_boundary() {
        let long = "ã".repeat(MAX_TERM_CHARS + 40);
        let got = normalise(&terms(&[&long]));
        assert_eq!(got[0].chars().count(), MAX_TERM_CHARS);
    }

    #[test]
    fn a_list_longer_than_the_cap_stops_at_it() {
        let many: Vec<String> = (0..MAX_TERMS + 30).map(|i| format!("term{i}")).collect();
        assert_eq!(normalise(&many).len(), MAX_TERMS);
    }

    /// No vocabulary means the prompt is exactly what the live pass was already
    /// sending — the guarantee that a user who never opens this field sees no
    /// change at all.
    #[test]
    fn without_terms_the_prompt_is_the_continuation_unchanged() {
        assert_eq!(
            initial_prompt(&[], "…the end of the last sentence"),
            "…the end of the last sentence"
        );
        assert_eq!(initial_prompt(&[], ""), "");
    }

    #[test]
    fn the_vocabulary_comes_first_and_the_continuation_last() {
        let p = initial_prompt(&terms(&["Vesper", "Yef"]), "e aí eu disse");
        assert_eq!(p, "Vesper, Yef. e aí eu disse");
    }

    #[test]
    fn a_whole_recording_pass_has_no_continuation_to_append() {
        assert_eq!(initial_prompt(&terms(&["Vesper"]), ""), "Vesper.");
    }

    /// The budget is what keeps whisper from truncating the prompt itself, and a
    /// truncation there would drop terms with nothing to say which.
    #[test]
    fn the_vocabulary_half_stays_inside_its_budget() {
        let many: Vec<String> = (0..MAX_TERMS)
            .map(|i| format!("termolongo{i:03}"))
            .collect();
        let normalised = normalise(&many);
        let p = initial_prompt(&normalised, "");
        assert!(
            p.chars().count() <= MAX_VOCAB_CHARS + 1,
            "{}",
            p.chars().count()
        );
        // Whole terms only — the last one in is complete, never half a word.
        assert!(p
            .trim_end_matches('.')
            .split(", ")
            .all(|t| normalised.iter().any(|n| n == t)));
    }
}
