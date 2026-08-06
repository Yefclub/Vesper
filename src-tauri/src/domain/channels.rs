//! Which of the two capture channels a recording listens to.

use crate::domain::settings::AppSettings;
use serde::{Deserialize, Serialize};

/// The pair of switches a recording is started with.
///
/// Kept apart from the device ids beside them in `AppSettings`, and
/// deliberately: `None` on `mic_device_id` has always meant "whichever device
/// the system calls default", so there was no value left in it to mean "none at
/// all". A channel switched off here keeps pointing at the device it was
/// pointing at, and gets it back the moment it is switched on again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSelection {
    pub me: bool,
    pub others: bool,
}

impl ChannelSelection {
    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            me: settings.capture_me,
            others: settings.capture_others,
        }
    }

    /// Whether this selection would record anything at all.
    pub fn records_anything(self) -> bool {
        self.me || self.others
    }
}

/// Both, which is what every recording did before the switches existed.
impl Default for ChannelSelection {
    fn default() -> Self {
        Self {
            me: true,
            others: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder is constructed with this before any settings are read, so
    /// the default has to be the behaviour that predates the switches.
    #[test]
    fn the_default_captures_both_channels() {
        let both = ChannelSelection::default();
        assert!(both.me && both.others);
        assert!(both.records_anything());
    }

    #[test]
    fn a_selection_with_neither_channel_records_nothing() {
        assert!(!ChannelSelection {
            me: false,
            others: false,
        }
        .records_anything());
    }

    /// One channel is a recording. Half a meeting is what the feature is for.
    #[test]
    fn one_channel_is_still_a_recording() {
        assert!(ChannelSelection {
            me: true,
            others: false,
        }
        .records_anything());
        assert!(ChannelSelection {
            me: false,
            others: true,
        }
        .records_anything());
    }

    #[test]
    fn the_switches_come_from_settings() {
        let s = AppSettings {
            capture_me: false,
            capture_others: true,
            ..AppSettings::default()
        };
        assert_eq!(
            ChannelSelection::from_settings(&s),
            ChannelSelection {
                me: false,
                others: true,
            }
        );
    }
}
