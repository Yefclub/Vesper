//! A channel that was asked to record and is hearing nothing.
//!
//! The failure this exists for looked like the application working. A meeting
//! came back with the other participants missing entirely — the file was there,
//! the right length, the user's own voice in it — and nobody found out until
//! afterwards. There was no error, because from the recorder's point of view
//! nothing went wrong: a stream that opens and delivers frames of zeros is
//! indistinguishable from a quiet room.
//!
//! It is distinguishable over time, though. A room is quiet in gaps; a channel
//! that is not connected to anything is quiet in every single sample. So the
//! rule is duration rather than level: a selected channel whose peak has never
//! once left the floor after this long is not a quiet room, and the user is
//! told while they can still do something about it.

/// How long a selected channel may hear literally nothing before it is worth
/// interrupting somebody over.
///
/// Long enough that the silence before anybody has spoken does not trip it,
/// short enough that the meeting is still worth saving. A false alarm costs a
/// banner; a missed one costs the meeting.
pub const DEAF_AFTER_MS: u64 = 20_000;

/// The level a peak has to clear to count as sound at all.
///
/// Not "silence in the room" — this is the floor below which a channel cannot
/// be told apart from one receiving nothing. Room tone through a live
/// microphone clears it comfortably.
const FLOOR: f32 = 0.002;

/// What the window should be told about the two channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Deaf {
    pub me: bool,
    pub others: bool,
}

impl Deaf {
    pub fn any(self) -> bool {
        self.me || self.others
    }
}

/// Which selected channels have heard nothing at all for long enough to say so.
///
/// `heard_*_ms` is when that channel last had a peak above the floor, on the
/// recorder's own clock; `None` means never. A channel that is not being
/// recorded is never deaf — nobody asked it to hear anything.
pub fn deaf_channels(
    now_ms: u64,
    capture_me: bool,
    capture_others: bool,
    heard_me_ms: Option<u64>,
    heard_others_ms: Option<u64>,
) -> Deaf {
    // Saturating for the reason the silence guard saturates: `now_ms` is the
    // recorder's elapsed clock and does not advance while paused, so a pause can
    // leave it behind a stamp taken before it.
    let quiet_for = |heard: Option<u64>| match heard {
        Some(t) => now_ms.saturating_sub(t),
        None => now_ms,
    };
    Deaf {
        me: capture_me && quiet_for(heard_me_ms) >= DEAF_AFTER_MS,
        others: capture_others && quiet_for(heard_others_ms) >= DEAF_AFTER_MS,
    }
}

/// Whether a peak counts as having heard something.
pub fn heard(peak: f32) -> bool {
    peak > FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_that_never_heard_anything_is_deaf() {
        let d = deaf_channels(DEAF_AFTER_MS, true, true, None, None);
        assert!(d.me && d.others && d.any());
    }

    #[test]
    fn under_the_threshold_nobody_is_warned() {
        let d = deaf_channels(DEAF_AFTER_MS - 1, true, true, None, None);
        assert!(!d.any(), "warned before the threshold");
    }

    /// The case that must not warn: a meeting nobody has spoken in yet. The
    /// channel heard something once, so it is connected — the room is merely
    /// quiet, which is the silence guard's business rather than this one's.
    #[test]
    fn a_channel_that_heard_something_is_not_deaf() {
        let d = deaf_channels(
            DEAF_AFTER_MS + 5_000,
            true,
            true,
            Some(10_000),
            Some(10_000),
        );
        assert!(!d.any());
    }

    /// The exact shape of the failure this was written for: the microphone
    /// worked and the system audio did not, so the recording looked fine until
    /// somebody read it.
    #[test]
    fn one_channel_can_be_deaf_alone() {
        let d = deaf_channels(60_000, true, true, Some(59_000), None);
        assert!(!d.me && d.others);
    }

    #[test]
    fn a_channel_that_is_off_is_never_deaf() {
        let d = deaf_channels(60_000, false, false, None, None);
        assert!(!d.any(), "warned about a channel nobody asked to record");
    }

    /// The recorder's clock stands still while paused, so a peak stamped before
    /// a pause can sit ahead of `now`. Wrapping there would read as an enormous
    /// quiet and warn instantly.
    #[test]
    fn a_clock_that_went_backwards_warns_nobody() {
        let d = deaf_channels(5_000, true, true, Some(30_000), Some(30_000));
        assert!(!d.any());
    }

    #[test]
    fn the_floor_admits_room_tone_and_rejects_nothing_at_all() {
        assert!(!heard(0.0));
        assert!(!heard(0.0005));
        assert!(heard(0.01));
    }
}
