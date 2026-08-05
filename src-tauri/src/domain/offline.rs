//! One switch that means the machine talks to nobody.
//!
//! "Cloud optional" is only a promise if it can be enforced, and today it is
//! enforced by the user remembering which provider they picked. Airplane mode
//! is the version of that promise a person can point at: turned on, nothing
//! this application does reaches the network, and the paths that would have are
//! refused with a sentence naming the switch rather than a timeout.

use serde::{Deserialize, Serialize};

/// What a blocked call was trying to do.
///
/// Carried so the refusal can say which thing did not happen. "Offline mode is
/// on" tells a user nothing when three different features are failing at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Egress {
    /// A summary, chat answer or title from a cloud model.
    CloudLlm,
    /// Transcription by a cloud provider.
    CloudStt,
    /// Fetching a model, or the CUDA pack, from the catalogue.
    Download,
    /// Asking whether a new version of the application exists.
    UpdateCheck,
}

impl Egress {
    /// What to tell the user, naming the thing that did not happen.
    ///
    /// Deliberately not "network error": a refusal the user configured is not a
    /// failure, and reading it as one sends them looking for a broken
    /// connection that is working perfectly.
    pub fn refusal(self) -> &'static str {
        match self {
            Egress::CloudLlm => {
                "Offline mode is on, so the summary was not sent to a cloud model. \
                 Turn it off in Settings, or choose a local model."
            }
            Egress::CloudStt => {
                "Offline mode is on, so the audio was not sent for transcription. \
                 Turn it off in Settings, or choose a local model."
            }
            Egress::Download => {
                "Offline mode is on, so nothing was downloaded. \
                 Turn it off in Settings to fetch this."
            }
            Egress::UpdateCheck => "Offline mode is on, so Vesper did not check for a new version.",
        }
    }
}

/// Whether this may leave the machine.
///
/// A single function rather than a boolean read at each call site: the point of
/// the switch is that there is one place to be sure about, and a site that
/// forgets to check is a silent hole in the only claim the product makes.
pub fn allowed(offline: bool, _what: Egress) -> bool {
    !offline
}

/// The refusal to return, or `None` when the call may proceed.
pub fn refuse(offline: bool, what: Egress) -> Option<String> {
    (!allowed(offline, what)).then(|| what.refusal().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_leaves_while_it_is_on() {
        for what in [
            Egress::CloudLlm,
            Egress::CloudStt,
            Egress::Download,
            Egress::UpdateCheck,
        ] {
            assert!(!allowed(true, what), "{what:?} escaped offline mode");
            assert!(refuse(true, what).is_some());
        }
    }

    #[test]
    fn everything_is_allowed_while_it_is_off() {
        for what in [
            Egress::CloudLlm,
            Egress::CloudStt,
            Egress::Download,
            Egress::UpdateCheck,
        ] {
            assert!(allowed(false, what));
            assert!(refuse(false, what).is_none());
        }
    }

    /// Each refusal names what did not happen. Four features failing with one
    /// sentence between them is a support question, not an explanation.
    #[test]
    fn each_refusal_says_which_thing_was_stopped() {
        let messages = [
            Egress::CloudLlm.refusal(),
            Egress::CloudStt.refusal(),
            Egress::Download.refusal(),
            Egress::UpdateCheck.refusal(),
        ];
        for (i, a) in messages.iter().enumerate() {
            for b in messages.iter().skip(i + 1) {
                assert_ne!(a, b, "two refusals read the same");
            }
            assert!(a.contains("Offline mode"), "{a}");
        }
    }
}
