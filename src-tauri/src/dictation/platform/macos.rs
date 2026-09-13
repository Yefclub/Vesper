//! macOS: the frontmost application, written into through the accessibility
//! attribute for selected text, and pasted into where that is not taken — with
//! the clipboard put back afterwards.
//!
//! Through `osascript` rather than bindings to the Accessibility framework, and
//! for a reason about this repository as much as the platform: no CI job builds
//! for macOS, and a process call is code that compiles wherever it is checked.
//!
//! Every script arrives on stdin and is ASCII: the dictated text never sits in
//! the process table, and no text encoding stands between it and the script.
//! Only words and counts come back — never what a field or the clipboard holds.

use crate::dictation::encode::{applescript_literal, js_literal};
use crate::dictation::verdict::{
    ax_verdict, count, pasted, permission_denied, AxVerdict, TypingClaim,
};
use crate::domain::dictation::{FailureReason, InsertionOutcome, Target};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Clone, Copy)]
enum Language {
    AppleScript,
    JavaScript,
}

/// Runs a script. A failure's text is only ever matched against, never logged.
fn osascript(language: Language, script: &str) -> Result<String, String> {
    let mut command = Command::new("/usr/bin/osascript");
    if let Language::JavaScript = language {
        command.args(["-l", "JavaScript"]);
    }
    let mut child = command
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "osascript took no input".to_string())?
        .write_all(script.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

pub fn capture_target() -> Option<Target> {
    let pid = osascript(
        Language::AppleScript,
        "with timeout of 3 seconds\n\
         tell application \"System Events\" to get unix id of first application process whose frontmost is true\n\
         end timeout",
    )
    .ok()?
    .parse::<u32>()
    .ok()?;
    // Only the process: System Events names no window or control a later call
    // could compare, and a window id guessed here would be a check that passes
    // for the wrong window.
    Some(Target {
        window: 0,
        control: 0,
        process: pid,
        process_started: 0,
    })
}

/// Shift, Control, Option and Command in `NSEvent.modifierFlags`.
const MODIFIERS_SCRIPT: &str = "ObjC.import('AppKit');\n\
    function held() { return ($.NSEvent.modifierFlags & 0x1E0000) !== 0; }\n\
    const deadline = Date.now() + 2000;\n\
    while (held() && Date.now() < deadline) delay(0.02);\n\
    held() ? 'held' : 'up'";

/// Waits for the shortcut's keys to be up. A machine that cannot be asked is
/// taken as the keys being up: this is a courtesy to the shortcut, not a check
/// the insertion depends on.
pub fn wait_for_modifiers() -> bool {
    !matches!(
        osascript(Language::JavaScript, MODIFIERS_SCRIPT).as_deref(),
        Ok("held")
    )
}

/// Counts in a field by splitting on the text, the AppleScript idiom for it.
const COUNT_OF: &str = "on countOf(needle, hay)\n\
    set saved to AppleScript's text item delimiters\n\
    set AppleScript's text item delimiters to needle\n\
    set n to (count of text items of hay) - 1\n\
    set AppleScript's text item delimiters to saved\n\
    return n\n\
    end countOf\n\
    on readValue(el)\n\
    tell application \"System Events\"\n\
    try\n\
    set v to value of attribute \"AXValue\" of el\n\
    on error\n\
    return missing value\n\
    end try\n\
    end tell\n\
    if class of v is text then return v\n\
    return missing value\n\
    end readValue\n";

/// Writes `text` over the selection of the focused element of `pid`, when that
/// element says the selection can be written, and reads the field before and
/// after. Its answers are the words `ax_verdict` knows.
fn write_script(pid: u32, text: &str) -> String {
    format!(
        "{COUNT_OF}\
         set t to {literal}\n\
         with timeout of 5 seconds\n\
         tell application \"System Events\"\n\
         try\n\
         set el to value of attribute \"AXFocusedUIElement\" of (first application process whose unix id is {pid})\n\
         on error number n\n\
         if n is -1719 or n is -25211 or n is -1743 then error number n\n\
         return \"no_target\"\n\
         end try\n\
         try\n\
         if value of attribute \"AXSubrole\" of el is \"AXSecureTextField\" then return \"secure\"\n\
         end try\n\
         set canWrite to false\n\
         try\n\
         set canWrite to settable of attribute \"AXSelectedText\" of el\n\
         end try\n\
         end tell\n\
         set before to my readValue(el)\n\
         if not canWrite then\n\
         if before is missing value then return \"paste -1\"\n\
         return \"paste \" & (my countOf(t, before))\n\
         end if\n\
         tell application \"System Events\" to set value of attribute \"AXSelectedText\" of el to t\n\
         delay 0.25\n\
         set after to my readValue(el)\n\
         end timeout\n\
         if before is missing value or after is missing value then return \"unconfirmed\"\n\
         set b to my countOf(t, before)\n\
         if (my countOf(t, after)) > b then return \"inserted\"\n\
         return \"paste \" & b\n",
        literal = applescript_literal(text),
    )
}

/// How many times the focused field of `pid` holds `text`, or `-1`.
fn count_script(pid: u32, text: &str) -> String {
    format!(
        "{COUNT_OF}\
         set t to {literal}\n\
         with timeout of 5 seconds\n\
         tell application \"System Events\"\n\
         try\n\
         set el to value of attribute \"AXFocusedUIElement\" of (first application process whose unix id is {pid})\n\
         on error\n\
         return -1\n\
         end try\n\
         end tell\n\
         end timeout\n\
         set v to my readValue(el)\n\
         if v is missing value then return -1\n\
         return my countOf(t, v)\n",
        literal = applescript_literal(text),
    )
}

/// Pastes `text` with Cmd+V and puts the clipboard back.
///
/// Every item and type on the clipboard is copied out first — images and files
/// as well as text — and nothing is touched if that fails. The text goes on it
/// marked transient and concealed, which clipboard managers take as "do not
/// keep". The snapshot comes back only if nothing else has written to the
/// clipboard since: anything that has is newer than what the user had.
fn paste_script(text: &str) -> String {
    format!(
        "ObjC.import('AppKit');\n\
         const TEXT = {literal};\n\
         const pb = $.NSPasteboard.generalPasteboard;\n\
         function snapshot() {{\n\
           const saved = [];\n\
           const items = pb.pasteboardItems;\n\
           if (items.isNil()) return saved;\n\
           for (let i = 0; i < items.count; i++) {{\n\
             const item = items.objectAtIndex(i);\n\
             const types = item.types;\n\
             const entry = [];\n\
             for (let j = 0; j < types.count; j++) {{\n\
               const type = types.objectAtIndex(j);\n\
               const data = item.dataForType(type);\n\
               if (!data.isNil()) entry.push([type, data]);\n\
             }}\n\
             saved.push(entry);\n\
           }}\n\
           return saved;\n\
         }}\n\
         function restore(saved) {{\n\
           pb.clearContents;\n\
           const out = $.NSMutableArray.array;\n\
           for (const entry of saved) {{\n\
             const item = $.NSPasteboardItem.alloc.init;\n\
             for (const [type, data] of entry) item.setDataForType(data, type);\n\
             out.addObject(item);\n\
           }}\n\
           if (out.count > 0) pb.writeObjects(out);\n\
         }}\n\
         let saved;\n\
         try {{ saved = snapshot(); }} catch (e) {{ throw new Error('clipboard_unsafe'); }}\n\
         pb.clearContents;\n\
         const ours = $.NSPasteboardItem.alloc.init;\n\
         ours.setStringForType($(TEXT), $.NSPasteboardTypeString);\n\
         ours.setDataForType($.NSData.data, 'org.nspasteboard.TransientType');\n\
         ours.setDataForType($.NSData.data, 'org.nspasteboard.ConcealedType');\n\
         const list = $.NSMutableArray.array;\n\
         list.addObject(ours);\n\
         pb.writeObjects(list);\n\
         const mine = pb.changeCount;\n\
         try {{\n\
           Application('System Events').keystroke('v', {{ using: 'command down' }});\n\
         }} catch (e) {{\n\
           restore(saved);\n\
           throw e;\n\
         }}\n\
         delay(0.5);\n\
         const outcome = pb.changeCount === mine ? (restore(saved), 'restored') : 'left';\n\
         outcome",
        literal = js_literal(text),
    )
}

pub fn insert(target: Target, text: &str, claim: &TypingClaim) -> InsertionOutcome {
    if !claim.begin_typing() {
        return InsertionOutcome::Failed(FailureReason::Timeout);
    }
    let verdict = match osascript(Language::AppleScript, &write_script(target.process, text)) {
        Ok(out) => ax_verdict(&out),
        Err(e) if permission_denied(&e) => {
            return InsertionOutcome::Failed(FailureReason::PermissionDenied)
        }
        Err(_) => return InsertionOutcome::Failed(FailureReason::Failed),
    };
    match verdict {
        Some(AxVerdict::Inserted) => InsertionOutcome::Inserted,
        Some(AxVerdict::NoTarget) => InsertionOutcome::Failed(FailureReason::NoEditableTarget),
        Some(AxVerdict::SecureInput) => InsertionOutcome::Failed(FailureReason::SecureInput),
        // Written and unreadable, or an answer outside the script's words: the
        // text may already be in the field, so nothing else is tried.
        Some(AxVerdict::Unconfirmed) | None => InsertionOutcome::Unconfirmed,
        Some(AxVerdict::Paste { before }) => {
            match osascript(Language::JavaScript, &paste_script(text)) {
                Ok(_) => {
                    let after =
                        osascript(Language::AppleScript, &count_script(target.process, text))
                            .ok()
                            .and_then(|out| count(&out))
                            .flatten();
                    pasted(before, after)
                }
                Err(e) if e.contains("clipboard_unsafe") => {
                    InsertionOutcome::Failed(FailureReason::ClipboardUnsafe)
                }
                Err(e) if permission_denied(&e) => {
                    InsertionOutcome::Failed(FailureReason::PermissionDenied)
                }
                // The keystroke may have gone before the failure.
                Err(_) => InsertionOutcome::Unconfirmed,
            }
        }
    }
}

pub fn can_insert() -> bool {
    true
}

pub fn shortcut_refusal() -> Option<&'static str> {
    None
}

pub fn floats_indicator() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPOKEN: &str = "ol\u{e1} \"mundo\" \\ fim\n\u{1f600}";

    /// The dictation reaches a script only as an escaped literal: no raw quote
    /// can close it, and nothing outside ASCII can be misread on the way in.
    #[test]
    fn every_script_carries_the_text_only_as_an_ascii_literal() {
        for script in [
            write_script(42, SPOKEN),
            count_script(42, SPOKEN),
            paste_script(SPOKEN),
        ] {
            assert!(script.is_ascii());
            assert!(!script.contains("\"mundo\""));
        }
        assert!(write_script(42, SPOKEN).contains("unix id is 42"));
    }

    /// Only the words `ax_verdict` knows come back from the write script.
    #[test]
    fn the_write_script_answers_in_the_parsed_vocabulary() {
        let script = write_script(1, "x");
        for word in [
            "\"no_target\"",
            "\"secure\"",
            "\"unconfirmed\"",
            "\"inserted\"",
            "\"paste ",
        ] {
            assert!(script.contains(word), "{word}");
        }
        assert_eq!(
            ax_verdict("paste 0"),
            Some(AxVerdict::Paste { before: Some(0) })
        );
    }

    /// The clipboard is put back only when nothing else has written to it, and
    /// what is put on it is marked for clipboard managers to skip.
    #[test]
    fn the_paste_script_guards_the_clipboard() {
        let script = paste_script("x");
        assert!(script.contains("pb.changeCount === mine"));
        assert!(script.contains("org.nspasteboard.TransientType"));
        assert!(script.contains("org.nspasteboard.ConcealedType"));
        assert!(script.contains("clipboard_unsafe"));
    }
}
