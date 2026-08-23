//! A channel that was asked to record and has never heard anything.
//!
//! The failure this exists for looked like the application working. A meeting
//! came back with the other participants missing entirely — the file was there,
//! the right length, the user's own voice in it — and nobody found out until
//! afterwards. There was no error, because from the recorder's point of view
//! nothing had gone wrong: a stream that opens and delivers frames of zeros is
//! indistinguishable from a quiet room.
//!
//! Indistinguishable in a single sample, at least. Over a whole recording the
//! two separate: a room goes quiet in gaps, and an input connected to nothing is
//! quiet in every sample it will ever produce. So this reports one state only —
//! **never heard anything at all** — and leaves every other kind of quiet to the
//! silence guard.

/// How long a dead channel may go unreported when the other one is working.
///
/// The other channel is the evidence, not the clock: one input delivering audio
/// while the other has produced literally nothing is not a quiet room, it is a
/// broken input. Twenty seconds is long enough to be sure and short enough that
/// the meeting is still worth saving.
pub const DEAF_AFTER_MS: u64 = 20_000;

/// How long BOTH channels may hear nothing before it is worth saying so.
///
/// Much longer, because there is nothing to lean on: two silent channels are a
/// broken setup and also a call nobody has spoken in yet, and no amount of
/// looking at the audio tells them apart. A minute and a half in which not one
/// person has made a sound is rare; a pair of dead inputs staying dead that long
/// is certain.
pub const SILENT_PAIR_MS: u64 = 90_000;

/// The level a peak has to clear to count as sound at all.
///
/// Not "silence in the room" — this is the floor below which a channel cannot be
/// told apart from one receiving nothing. Room tone through a live microphone
/// clears it comfortably.
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

/// Which selected channels have never produced a sound, for long enough to say
/// so.
///
/// **A channel that has heard something is never deaf**, however quiet it goes
/// afterwards. That is the line between this and the silence guard: an input
/// that has delivered audio is connected, and a room it then stops hearing is a
/// room with nobody talking in it. Reporting that here would put a red banner
/// over every pause in every meeting.
///
/// A channel that is not being recorded is never deaf either — nobody asked it
/// to hear anything, and its silence is not evidence about the other one.
/// `evidence_at_ms` is when the first selected channel produced its first
/// sound, on the recorder's own clock — `None` while neither has.
pub fn deaf_channels(
    now_ms: u64,
    capture_me: bool,
    capture_others: bool,
    heard_me: bool,
    heard_others: bool,
    evidence_at_ms: Option<u64>,
) -> Deaf {
    let silent = |on: bool, heard: bool| on && !heard;
    let working = |on: bool, heard: bool| on && heard;
    let have_evidence = working(capture_me, heard_me) || working(capture_others, heard_others);
    // One silent channel beside a working one is proof, and the clock starts at
    // the proof rather than at the recording. A call where nobody speaks for
    // twenty seconds and then somebody does would otherwise be accused the
    // instant the first word arrives, having been given no time at all to show
    // that the other channel works too.
    //
    // Two silent channels have nothing to be measured against, so only elapsed
    // time decides — see `SILENT_PAIR_MS`, counted from the start because there
    // is no other event to count from.
    let long_enough = if have_evidence {
        match evidence_at_ms {
            // Saturating: the recorder's clock stands still while paused, so a
            // stamp taken before a pause can sit ahead of `now`.
            Some(at) => now_ms.saturating_sub(at) >= DEAF_AFTER_MS,
            None => false,
        }
    } else {
        now_ms >= SILENT_PAIR_MS
    };
    Deaf {
        me: silent(capture_me, heard_me) && long_enough,
        others: silent(capture_others, heard_others) && long_enough,
    }
}

/// Whether a peak counts as having heard something.
pub fn heard(peak: f32) -> bool {
    peak > FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape of the failure this was written for: the microphone
    /// delivered audio and the system channel never did, so the recording looked
    /// fine until somebody read it. One working channel is what makes the other
    /// one's silence evidence rather than a guess.
    #[test]
    fn a_dead_channel_beside_a_working_one_is_reported() {
        let d = deaf_channels(DEAF_AFTER_MS, true, true, true, false, Some(0));
        assert!(!d.me && d.others && d.any());
    }

    #[test]
    fn under_the_threshold_nobody_is_warned() {
        let d = deaf_channels(DEAF_AFTER_MS - 1, true, true, true, false, Some(0));
        assert!(!d.any(), "warned before the threshold");
    }

    /// The clock starts at the proof, not at the recording. A call where nobody
    /// speaks for a while and then somebody does must not be accused the instant
    /// the first word lands — the other channel has had no chance to show it
    /// works.
    #[test]
    fn the_wait_runs_from_the_first_sound_not_from_the_start() {
        let spoke_at = 300_000;
        assert!(
            !deaf_channels(spoke_at + 1, true, true, true, false, Some(spoke_at)).any(),
            "accused a channel a millisecond after the first word of the meeting"
        );
        assert!(
            deaf_channels(
                spoke_at + DEAF_AFTER_MS,
                true,
                true,
                true,
                false,
                Some(spoke_at)
            )
            .others
        );
    }

    /// The false alarm that matters, and the reason two silent channels wait so
    /// much longer: somebody joins a call early and nobody has spoken yet. Both
    /// inputs are fine and both are quiet, and there is nothing in the audio to
    /// tell that apart from two dead ones.
    #[test]
    fn a_meeting_nobody_has_spoken_in_yet_is_not_reported() {
        for t in [DEAF_AFTER_MS, 45_000, SILENT_PAIR_MS - 1] {
            assert!(
                !deaf_channels(t, true, true, false, false, None).any(),
                "warned at {t}ms about a call nobody had spoken in"
            );
        }
    }

    /// It is reported eventually. A minute and a half without one sound from
    /// either input is a broken setup rather than a polite room.
    #[test]
    fn two_silent_channels_are_reported_in_the_end() {
        let d = deaf_channels(SILENT_PAIR_MS, true, true, false, false, None);
        assert!(d.me && d.others);
    }

    /// Once a channel has delivered audio it is connected, and a room it then
    /// stops hearing belongs to the silence guard.
    #[test]
    fn a_channel_that_went_quiet_after_working_is_never_deaf() {
        let d = deaf_channels(600_000, true, true, true, true, Some(1_000));
        assert!(!d.any());
    }

    #[test]
    fn a_channel_that_is_off_is_never_deaf() {
        let d = deaf_channels(600_000, false, false, false, false, None);
        assert!(!d.any(), "warned about a channel nobody asked to record");
    }

    /// A channel that is off must not lend its silence as evidence against the
    /// one that is on: recording the microphone alone is an ordinary choice, and
    /// the system channel being quiet is exactly what was asked for.
    #[test]
    fn an_off_channel_is_not_evidence_about_the_other() {
        assert!(
            !deaf_channels(DEAF_AFTER_MS + 1_000, true, false, false, false, None).any(),
            "the short wait was used with no working channel to justify it"
        );
        assert!(deaf_channels(SILENT_PAIR_MS, true, false, false, false, None).me);
    }

    #[test]
    fn the_floor_admits_room_tone_and_rejects_nothing_at_all() {
        assert!(!heard(0.0));
        assert!(!heard(0.0005));
        assert!(heard(0.01));
    }
}
