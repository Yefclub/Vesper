//! Text as each platform wants to be handed it.
//!
//! Pure, so the part of insertion that can be wrong without a desktop in front
//! of it is tested without one.

/// One keyboard event carrying a UTF-16 code unit, pressed or released.
///
/// `KEYEVENTF_UNICODE` takes code units rather than characters, so a character
/// outside the Basic Multilingual Plane — an emoji — is two units and four
/// events, and sending only the first leaves half a surrogate in the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnicodeKey {
    pub unit: u16,
    pub up: bool,
}

pub fn unicode_keys(text: &str) -> Vec<UnicodeKey> {
    text.encode_utf16()
        .flat_map(|unit| {
            [
                UnicodeKey { unit, up: false },
                UnicodeKey { unit, up: true },
            ]
        })
        .collect()
}

/// `text` in pieces of at most `max_units` UTF-16 units, never splitting a
/// character. A target that reads its input slower than it arrives drops what
/// overflows, so a long dictation goes in pieces with a breath between them.
pub fn batches(text: &str, max_units: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (i, c) in text.char_indices() {
        let n = c.len_utf16();
        if units > 0 && units + n > max_units {
            out.push(&text[start..i]);
            start = i;
            units = 0;
        }
        units += n;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// A string as an AppleScript expression, in ASCII.
///
/// The dictated text becomes part of a script the system runs. Without the
/// escaping a transcript containing a quote closes the literal early and what
/// follows it is script — a spoken sentence would run as code. Anything outside
/// printable ASCII is joined in as `character id`, so no text encoding stands
/// between an accented word and the script that types it.
pub fn applescript_literal(text: &str) -> String {
    fn close(run: &mut String, parts: &mut Vec<String>) {
        if !run.is_empty() {
            parts.push(format!("\"{}\"", std::mem::take(run)));
        }
    }
    let mut parts = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        match c {
            '\\' => run.push_str("\\\\"),
            '"' => run.push_str("\\\""),
            // `return` is AppleScript's own line break, joined back in.
            '\n' | '\r' => {
                close(&mut run, &mut parts);
                parts.push("return".to_string());
            }
            c if c.is_ascii() && !c.is_ascii_control() => run.push(c),
            c => {
                close(&mut run, &mut parts);
                parts.push(format!("(character id {})", u32::from(c)));
            }
        }
    }
    close(&mut run, &mut parts);
    // Always opening on a string, so the expression reads as text from its
    // first term.
    if !parts.first().is_some_and(|p| p.starts_with('"')) {
        parts.insert(0, "\"\"".to_string());
    }
    parts.join(" & ")
}

/// A string as a JavaScript literal, in ASCII: quotes and backslashes escaped,
/// and every other character outside printable ASCII as its UTF-16 escape.
pub fn js_literal(text: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for unit in text.encode_utf16() {
        match unit {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from(unit as u8)),
            _ => {
                let _ = write!(out, "\\u{unit:04x}");
            }
        }
    }
    out.push('"');
    out
}

/// How many times `needle` occurs in `haystack`, without overlaps.
///
/// An insertion is confirmed by the field holding the text one more time than
/// it did before, not merely holding it: a field that already contained the
/// same words would otherwise confirm an insertion that never happened.
pub fn occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_unit_is_pressed_then_released() {
        assert_eq!(
            unicode_keys("\u{e9}"),
            vec![
                UnicodeKey {
                    unit: 0x00e9,
                    up: false
                },
                UnicodeKey {
                    unit: 0x00e9,
                    up: true
                },
            ]
        );
    }

    /// Both halves of a surrogate pair, in order — half of one is a broken
    /// character in the target.
    #[test]
    fn a_character_outside_the_bmp_is_two_units() {
        let keys = unicode_keys("\u{1f600}");
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0].unit, 0xd83d);
        assert_eq!(keys[2].unit, 0xde00);
    }

    #[test]
    fn batches_never_split_a_character() {
        assert_eq!(batches("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(batches("ab\u{1f600}", 3), vec!["ab", "\u{1f600}"]);
        assert_eq!(batches("", 4), Vec::<&str>::new());
        // A single character wider than the limit still goes, alone.
        assert_eq!(batches("\u{1f600}", 1), vec!["\u{1f600}"]);
    }

    /// A sentence that happens to contain script must stay a sentence.
    #[test]
    fn quotes_and_backslashes_cannot_close_the_literal() {
        assert_eq!(
            applescript_literal(r#"say "hi" \ now"#),
            r#""say \"hi\" \\ now""#
        );
        let hostile = r#"" & (do shell script "echo pwned") & ""#;
        let literal = applescript_literal(hostile);
        // Every quote from the input arrives escaped.
        assert_eq!(
            literal.matches('"').count(),
            literal.matches("\\\"").count() + 2
        );
    }

    #[test]
    fn line_breaks_become_returns() {
        assert_eq!(applescript_literal("a\nb"), r#""a" & return & "b""#);
        assert_eq!(applescript_literal("\nb"), r#""" & return & "b""#);
    }

    #[test]
    fn applescript_text_outside_ascii_goes_in_by_character_id() {
        assert_eq!(
            applescript_literal("ol\u{e1}"),
            r#""ol" & (character id 225)"#
        );
        assert_eq!(
            applescript_literal("\u{1f600}!"),
            r#""" & (character id 128512) & "!""#
        );
        assert_eq!(applescript_literal(""), r#""""#);
        assert!(applescript_literal("a\u{e7}\u{e3}o, cora\u{e7}\u{e3}o").is_ascii());
    }

    #[test]
    fn a_js_literal_is_ascii_and_cannot_be_closed_early() {
        assert_eq!(js_literal(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(js_literal("\u{e9}\n"), "\"\\u00e9\\u000a\"");
        // Both halves of the pair, which is how a JavaScript string holds it.
        assert_eq!(js_literal("\u{1f600}"), "\"\\ud83d\\ude00\"");
        // The separators JSON allows raw and older JavaScript does not.
        assert_eq!(js_literal("\u{2028}"), "\"\\u2028\"");
        let hostile = js_literal(r#""); doShellScript("x"); (""#);
        assert!(hostile.is_ascii());
        assert_eq!(
            hostile.matches('"').count(),
            hostile.matches("\\\"").count() + 2
        );
    }

    #[test]
    fn occurrences_count_whole_matches() {
        assert_eq!(occurrences("ok ok ok", "ok"), 3);
        assert_eq!(occurrences("hello", "bye"), 0);
        assert_eq!(occurrences("anything", ""), 0);
    }
}
