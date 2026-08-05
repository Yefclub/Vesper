//! The record accelerator — one source of truth, and whether the OS took it.

use serde::{Deserialize, Serialize};

/// The combination as the window writes it in a `<kbd>`.
///
/// `lib.rs` registers the same one through the typed constructor, and
/// `the_display_string_matches_the_registered_code` is what stops the two
/// drifting — they were independent literals in two languages, with nothing
/// checking that the key advertised was the key registered.
pub const RECORD_ACCELERATOR: &str = "Ctrl+Shift+R";

/// The combinations the user may choose between.
///
/// A fixed list, not a key-capture field. Capture sounds better and is worse
/// here: the WebView never sees a chord the OS has already given to another
/// application — which is the exact case this exists for — so it would happily
/// offer combinations that then fail to register. Every entry here is parsed by
/// the same code that registers it, and a test asserts that, so a choice that
/// cannot be registered cannot be offered.
///
/// Modifier-heavy on purpose. A bare function key is one other applications
/// take without asking, and taking it back from them is not this app's business.
pub const CHOICES: [&str; 6] = [
    "Ctrl+Shift+R",
    "Ctrl+Alt+R",
    "Ctrl+Shift+M",
    "Alt+Shift+R",
    "Ctrl+Shift+F9",
    "Ctrl+Alt+Space",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutStatus {
    pub registered: bool,
    pub accelerator: String,
    pub reason_key: Option<String>,
    /// What else the user may pick. Sent from here so the window does not keep
    /// a second copy of a list only this module can actually register.
    #[serde(default)]
    pub choices: Vec<String>,
}

/// Whether a string is one this application is willing to register.
///
/// The WebView is the trust boundary and a global accelerator is a system-wide
/// grab, so the answer is an allow-list rather than a parser: anything outside
/// `CHOICES` is refused before it reaches the OS.
pub fn is_offered(accelerator: &str) -> bool {
    CHOICES.contains(&accelerator)
}

/// The stored value, or the default when it is one this build no longer offers.
pub fn chosen_or_default(stored: &str) -> &'static str {
    CHOICES
        .iter()
        .find(|c| **c == stored)
        .copied()
        .unwrap_or(RECORD_ACCELERATOR)
}

/// Turn the registration result into something the window can render.
///
/// A combination another application already owns must not stop Vesper from
/// starting, so the error is reported rather than propagated. Reporting it only
/// to `tracing` was the other half of the problem: the empty state kept
/// advertising a key the OS had refused, with no way to learn otherwise — and
/// no way to pick a different one, which is what `choices` is for.
pub fn status_from<E>(result: Result<(), E>, accelerator: &str) -> ShortcutStatus {
    ShortcutStatus {
        registered: result.is_ok(),
        accelerator: accelerator.to_string(),
        reason_key: result.is_err().then(|| "shortcut.unavailable".to_string()),
        choices: CHOICES.iter().map(|c| c.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window reads these names off the IPC payload and nothing checks that
    /// contract across the boundary — the front end is a separate branch in
    /// another language. A rename or a stray `rename_all` would leave every test
    /// here green and the shortcut notice permanently blank.
    #[test]
    fn the_wire_shape_is_the_one_the_window_reads() {
        let taken =
            serde_json::to_value(status_from::<&str>(Err("taken"), RECORD_ACCELERATOR)).unwrap();
        assert_eq!(taken["registered"], serde_json::json!(false));
        assert_eq!(taken["accelerator"], serde_json::json!(RECORD_ACCELERATOR));
        assert_eq!(
            taken["reason_key"],
            serde_json::json!("shortcut.unavailable")
        );
        assert_eq!(taken["choices"][0], serde_json::json!(CHOICES[0]));

        let ok = serde_json::to_value(status_from::<&str>(Ok(()), "Ctrl+Alt+R")).unwrap();
        assert_eq!(ok["registered"], serde_json::json!(true));
        assert_eq!(ok["accelerator"], serde_json::json!("Ctrl+Alt+R"));
        assert_eq!(ok["reason_key"], serde_json::json!(null));
    }

    #[test]
    fn an_unregistered_shortcut_names_its_reason_key() {
        let s = status_from::<&str>(Err("already registered"), RECORD_ACCELERATOR);
        assert!(!s.registered);
        assert_eq!(s.reason_key.as_deref(), Some("shortcut.unavailable"));
        assert_eq!(s.accelerator, RECORD_ACCELERATOR);
    }

    #[test]
    fn a_registered_shortcut_has_no_reason() {
        let s = status_from::<&str>(Ok(()), RECORD_ACCELERATOR);
        assert!(s.registered);
        assert_eq!(s.reason_key, None);
        assert_eq!(s.accelerator, RECORD_ACCELERATOR);
    }

    /// A global accelerator is a system-wide grab and the request comes from the
    /// WebView. The allow-list is the whole check — nothing here parses
    /// arbitrary input into a `Shortcut`.
    #[test]
    fn only_offered_combinations_are_accepted() {
        assert!(is_offered("Ctrl+Shift+R"));
        assert!(is_offered("Ctrl+Alt+Space"));
        assert!(!is_offered("Ctrl+Q"));
        assert!(!is_offered(""));
        assert!(!is_offered("Ctrl+Shift+R "), "not trimmed for the caller");
    }

    /// A row written by a build that offered something this one does not must
    /// not leave the user with no shortcut at all.
    #[test]
    fn an_unknown_stored_value_falls_back_to_the_default() {
        assert_eq!(chosen_or_default("Ctrl+Alt+R"), "Ctrl+Alt+R");
        assert_eq!(
            chosen_or_default("Ctrl+Shift+Backspace"),
            RECORD_ACCELERATOR
        );
        assert_eq!(chosen_or_default(""), RECORD_ACCELERATOR);
    }

    /// Whether `RegisterHotKey` succeeds is a Win32 call against live
    /// per-machine state and cannot be tested here. What can be tested is that
    /// the string the window shows and the code the app registers are the same
    /// combination — for every choice, not just the default, because each one is
    /// a combination somebody can pick and then find does nothing.
    #[test]
    fn every_choice_is_a_combination_the_app_can_register() {
        use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
        assert_eq!(
            RECORD_ACCELERATOR.parse::<Shortcut>().unwrap(),
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyR)
        );
        for c in CHOICES {
            assert!(
                c.parse::<Shortcut>().is_ok(),
                "{c} is offered but cannot be registered"
            );
        }
    }

    /// The default has to be in the list, or the settings screen opens showing a
    /// value none of its options match.
    #[test]
    fn the_default_is_one_of_the_choices() {
        assert!(is_offered(RECORD_ACCELERATOR));
    }
}
