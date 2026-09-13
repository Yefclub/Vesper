//! Decisions the insertion adapters make, kept apart from the calls that gather
//! their inputs — so they are tested on every platform, not only on the one
//! whose API supplies them.

use crate::domain::dictation::InsertionOutcome;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Polls `done` until it answers true or `timeout` has passed. Whether it did.
pub fn wait_until(timeout: Duration, step: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(step);
    }
}

/// Whether any of `keycodes` is down in an X11 keymap: one bit per keycode,
/// least significant bit first. Keycode 0 is the padding of an unused slot.
pub fn any_key_down(keymap: &[u8], keycodes: impl IntoIterator<Item = u8>) -> bool {
    keycodes.into_iter().filter(|&k| k != 0).any(|k| {
        keymap
            .get(usize::from(k / 8))
            .is_some_and(|byte| byte & (1 << (k % 8)) != 0)
    })
}

/// UI Automation control types that never take typed text, from
/// `UIAutomationClient.h`: button, check box, hyperlink, image, list item, menu
/// bar, menu item, radio button, scroll bar, slider, tab item and tree item.
/// Typing at one of these is pressing its keyboard shortcuts — a file list
/// jumps, a mail list archives.
pub const NOT_TEXT_CONTROLS: [i32; 12] = [
    50000, 50002, 50005, 50006, 50007, 50010, 50011, 50013, 50014, 50015, 50019, 50024,
];

/// Whether the focused element refuses typed text, as far as UI Automation says.
///
/// Only what it states outright counts. An element that exposes nothing is
/// typed into: refusing those would refuse most applications, which describe
/// their fields to UI Automation poorly or not at all.
pub fn refuses_text(control_type: Option<i32>, read_only: Option<bool>) -> bool {
    read_only == Some(true) || control_type.is_some_and(|c| NOT_TEXT_CONTROLS.contains(&c))
}

/// Who reaches an insertion first: the thread about to type, or the timeout
/// giving up on it.
///
/// Exactly one of them wins. A timeout that has been reported can therefore
/// never be followed by the text turning up, and a retry of it can never type
/// it twice.
#[derive(Debug, Default)]
pub struct TypingClaim(AtomicU8);

const PENDING: u8 = 0;
const TYPING: u8 = 1;
const ABANDONED: u8 = 2;

impl TypingClaim {
    /// The typing side, just before the first key: whether it may type.
    pub fn begin_typing(&self) -> bool {
        self.0
            .compare_exchange(PENDING, TYPING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// The timeout side: whether typing had not begun, and now never will.
    pub fn give_up(&self) -> bool {
        self.0
            .compare_exchange(PENDING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

/// What the macOS accessibility script reports about writing into the focused
/// element. Numbers and words only cross back from it — never the field's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxVerdict {
    /// The field reads back holding the text once more than before.
    Inserted,
    /// Written, and the field cannot be read back. Nothing else may be tried:
    /// the text may already be in it.
    Unconfirmed,
    /// Not written — the element does not take text this way, or took it and
    /// holds nothing new, which is how Chromium and Electron fields answer.
    /// `before` is how many times the field held the text already, when it can
    /// be read.
    Paste { before: Option<usize> },
    /// Nothing has the keyboard.
    NoTarget,
    /// A password field has it, which takes no text from another application.
    SecureInput,
}

pub fn ax_verdict(output: &str) -> Option<AxVerdict> {
    let mut words = output.split_whitespace();
    match (words.next()?, words.next()) {
        ("inserted", None) => Some(AxVerdict::Inserted),
        ("unconfirmed", None) => Some(AxVerdict::Unconfirmed),
        ("no_target", None) => Some(AxVerdict::NoTarget),
        ("secure", None) => Some(AxVerdict::SecureInput),
        ("paste", Some(n)) => Some(AxVerdict::Paste { before: count(n)? }),
        _ => None,
    }
}

/// A count the scripts print, where `-1` means the field could not be read.
pub fn count(word: &str) -> Option<Option<usize>> {
    match word.trim().parse::<i64>().ok()? {
        -1 => Some(None),
        n => usize::try_from(n).ok().map(Some),
    }
}

/// A paste is confirmed by the field holding the text once more than it did
/// before it — and only when both readings exist.
pub fn pasted(before: Option<usize>, after: Option<usize>) -> InsertionOutcome {
    match (before, after) {
        (Some(b), Some(a)) if a > b => InsertionOutcome::Inserted,
        _ => InsertionOutcome::Unconfirmed,
    }
}

/// Whether an `osascript` failure is macOS refusing this app the permission to
/// control other applications, rather than anything the target did.
///
/// -1719 and -25211 are Accessibility not granted to UI scripting, 1002 is the
/// same refusal for keystrokes, and -1743 is Automation of System Events not
/// granted.
pub fn permission_denied(stderr: &str) -> bool {
    [
        "(-1719)",
        "(-25211)",
        "(1002)",
        "(-1743)",
        "assistive access",
        "not authorized",
        "not allowed",
    ]
    .iter()
    .any(|needle| stderr.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn waiting_stops_as_soon_as_the_condition_holds() {
        let calls = Cell::new(0);
        let done = wait_until(Duration::from_secs(1), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            calls.get() == 3
        });
        assert!(done);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn waiting_gives_up_at_the_timeout() {
        assert!(!wait_until(
            Duration::from_millis(20),
            Duration::from_millis(5),
            || false
        ));
    }

    #[test]
    fn a_keymap_bit_names_its_keycode() {
        let mut keymap = [0u8; 32];
        // Keycode 37 is byte 4, bit 5.
        keymap[4] = 1 << 5;
        assert!(any_key_down(&keymap, [50, 37]));
        assert!(!any_key_down(&keymap, [36, 38]));
        // An unused modifier slot is keycode 0, and is not a key.
        keymap[0] = 1;
        assert!(!any_key_down(&keymap, [0]));
    }

    #[test]
    fn only_a_stated_refusal_refuses() {
        assert!(refuses_text(None, Some(true)));
        assert!(refuses_text(Some(50000), None));
        // Edit, document and data item take text; an element that says nothing
        // is typed into.
        assert!(!refuses_text(Some(50004), Some(false)));
        assert!(!refuses_text(Some(50030), None));
        assert!(!refuses_text(Some(50029), None));
        assert!(!refuses_text(None, None));
    }

    #[test]
    fn a_timeout_and_the_typing_cannot_both_win() {
        let late = TypingClaim::default();
        assert!(late.give_up());
        assert!(!late.begin_typing());

        let typing = TypingClaim::default();
        assert!(typing.begin_typing());
        assert!(!typing.give_up());
    }

    #[test]
    fn the_accessibility_script_answers_in_a_closed_vocabulary() {
        assert_eq!(ax_verdict("inserted\n"), Some(AxVerdict::Inserted));
        assert_eq!(ax_verdict("unconfirmed"), Some(AxVerdict::Unconfirmed));
        assert_eq!(ax_verdict("no_target"), Some(AxVerdict::NoTarget));
        assert_eq!(ax_verdict("secure"), Some(AxVerdict::SecureInput));
        assert_eq!(
            ax_verdict("paste 2"),
            Some(AxVerdict::Paste { before: Some(2) })
        );
        assert_eq!(
            ax_verdict("paste -1"),
            Some(AxVerdict::Paste { before: None })
        );
        for junk in ["", "paste", "paste x", "inserted 3", "typed"] {
            assert_eq!(ax_verdict(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn a_paste_is_confirmed_only_by_two_readings() {
        assert_eq!(pasted(Some(0), Some(1)), InsertionOutcome::Inserted);
        assert_eq!(pasted(Some(1), Some(1)), InsertionOutcome::Unconfirmed);
        assert_eq!(pasted(None, Some(1)), InsertionOutcome::Unconfirmed);
        assert_eq!(pasted(Some(0), None), InsertionOutcome::Unconfirmed);
    }

    #[test]
    fn a_refused_permission_is_told_apart_from_a_failure() {
        assert!(permission_denied(
            "execution error: System Events got an error: osascript is not allowed assistive access. (-1719)"
        ));
        assert!(permission_denied(
            "Not authorized to send Apple events to System Events. (-1743)"
        ));
        assert!(permission_denied(
            "execution error: System Events got an error: osascript is not allowed to send keystrokes. (1002)"
        ));
        assert!(!permission_denied(
            "execution error: Can't get window 1. (-1728)"
        ));
    }
}
