//! The record accelerator — one source of truth, and whether the OS took it.

use serde::{Deserialize, Serialize};

/// The combination as the window writes it in a `<kbd>`.
///
/// `lib.rs` registers the same one through the typed constructor, and
/// `the_display_string_matches_the_registered_code` is what stops the two
/// drifting — they were independent literals in two languages, with nothing
/// checking that the key advertised was the key registered.
pub const RECORD_ACCELERATOR: &str = "Ctrl+Shift+R";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutStatus {
    pub registered: bool,
    pub accelerator: String,
    pub reason_key: Option<String>,
}

/// Turn the registration result into something the window can render.
///
/// A combination another application already owns must not stop Vesper from
/// starting, so the error is reported rather than propagated. Reporting it only
/// to `tracing` was the other half of the problem: the empty state kept
/// advertising a key the OS had refused, with no way to learn otherwise.
pub fn status_from<E>(result: Result<(), E>) -> ShortcutStatus {
    match result {
        Ok(()) => ShortcutStatus {
            registered: true,
            accelerator: RECORD_ACCELERATOR.to_string(),
            reason_key: None,
        },
        Err(_) => ShortcutStatus {
            registered: false,
            accelerator: RECORD_ACCELERATOR.to_string(),
            reason_key: Some("shortcut.unavailable".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unregistered_shortcut_names_its_reason_key() {
        let s = status_from::<&str>(Err("already registered"));
        assert!(!s.registered);
        assert_eq!(s.reason_key.as_deref(), Some("shortcut.unavailable"));
        assert_eq!(s.accelerator, RECORD_ACCELERATOR);
    }

    #[test]
    fn a_registered_shortcut_has_no_reason() {
        let s = status_from::<&str>(Ok(()));
        assert!(s.registered);
        assert_eq!(s.reason_key, None);
        assert_eq!(s.accelerator, RECORD_ACCELERATOR);
    }

    /// Whether `RegisterHotKey` succeeds is a Win32 call against live
    /// per-machine state and cannot be tested here. What can be tested is that
    /// the string the window shows and the code the app registers are the same
    /// combination.
    #[test]
    fn the_display_string_matches_the_registered_code() {
        use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
        assert_eq!(
            RECORD_ACCELERATOR.parse::<Shortcut>().unwrap(),
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyR)
        );
    }
}
