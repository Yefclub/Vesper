//! What to do when nobody has said anything for a long time.
//!
//! A recording left running over an empty room costs disk, costs battery, and
//! costs the user a meeting full of nothing to scroll past afterwards. It also
//! costs money on a cloud provider, which charges by the second of audio it is
//! handed whether or not there were words in it.
//!
//! The rule is deliberately not "stop after three minutes of quiet". Stopping a
//! recording is destructive in the sense that matters — the meeting ends and
//! the user has to notice and start another — so quiet first raises a question,
//! and only silence in the face of the question ends it.

/// Quiet long enough to ask whether anybody is still there.
pub const ASK_AFTER_MS: u64 = 3 * 60 * 1000;

/// How long the question stands before the answer is taken as "no".
///
/// Five minutes, and longer than the wait that raised it on purpose: somebody
/// who stepped out of the room is exactly the person this is about, and the
/// window has to be wide enough for them to come back to it.
pub const ANSWER_WINDOW_MS: u64 = 5 * 60 * 1000;

/// How long audio nobody has read may hold the stop back.
///
/// A recording must never end over audio nobody has looked at — somebody
/// speaking in the last seconds of the window, or an utterance a failing
/// provider handed back. But a grace period rather than a gate: a room with a
/// fan in it is above the segmenter's threshold forever, and a gate there would
/// mean the stop never comes at all. Thirty seconds is twice the segmenter's
/// own ceiling — long enough for anything actually spoken to have been cut,
/// transcribed, and to have answered the question by itself.
pub const UNREAD_GRACE_MS: u64 = 30_000;

/// The wait for an answer outlasts the wait that raised the question. Somebody
/// who stepped out of the room is the person this is about, and the window has
/// to be wide enough for them to come back to it.
///
/// A compile-time check rather than a test: it is a property of two constants,
/// and getting it wrong should stop the build rather than a test run.
const _: () = assert!(ANSWER_WINDOW_MS > ASK_AFTER_MS);

/// Where a recording stands with respect to the quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vigil {
    /// Somebody has spoken recently enough that there is nothing to ask.
    #[default]
    Listening,
    /// The question is on screen, raised at this many milliseconds into the
    /// recording.
    Asking { since_ms: u64 },
}

/// What the caller should do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Nothing.
    Nothing,
    /// Put the question on screen.
    Ask,
    /// Take the question down — somebody spoke.
    Dismiss,
    /// The question stood unanswered for its whole window. Stop.
    Stop,
}

/// Advance the vigil.
///
/// `quiet_for_ms` is how long it has been since the last speech, `now_ms` is the
/// recording's own clock, and `unread` says whether audio exists that nobody has
/// transcribed yet. All three come from the caller because all three are facts
/// about audio, and this module is the policy over them.
///
/// Speech resets everything, in either state. That is the whole reason the
/// question exists: it is not a countdown the user has to beat, it is a check
/// that somebody is there, and somebody speaking IS the answer — they do not
/// have to find a button.
pub fn advance(
    state: Vigil,
    quiet_for_ms: u64,
    now_ms: u64,
    speaking: bool,
    unread: bool,
) -> (Vigil, Act) {
    if speaking {
        return match state {
            Vigil::Listening => (Vigil::Listening, Act::Nothing),
            Vigil::Asking { .. } => (Vigil::Listening, Act::Dismiss),
        };
    }
    match state {
        Vigil::Listening if quiet_for_ms >= ASK_AFTER_MS => {
            (Vigil::Asking { since_ms: now_ms }, Act::Ask)
        }
        Vigil::Listening => (Vigil::Listening, Act::Nothing),
        Vigil::Asking { since_ms } => {
            // Saturating, and it matters: `now_ms` is the recorder's elapsed
            // clock, which does not advance while paused, so a pause during the
            // question can leave `now` behind `since`. A wrapping subtraction
            // there would read as an enormous elapsed time and stop the
            // recording instantly.
            let waited = now_ms.saturating_sub(since_ms);
            if waited >= ANSWER_WINDOW_MS
                && !(unread && waited < ANSWER_WINDOW_MS + UNREAD_GRACE_MS)
            {
                (Vigil::Listening, Act::Stop)
            } else {
                (state, Act::Nothing)
            }
        }
    }
}

/// The user answered the question. Whatever they said, they are there.
pub fn answered() -> Vigil {
    Vigil::Listening
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_under_the_threshold_says_nothing() {
        let (state, act) = advance(Vigil::Listening, ASK_AFTER_MS - 1, 10_000, false, false);
        assert_eq!(state, Vigil::Listening);
        assert_eq!(act, Act::Nothing);
    }

    #[test]
    fn three_minutes_of_quiet_raises_the_question() {
        let (state, act) = advance(Vigil::Listening, ASK_AFTER_MS, 200_000, false, false);
        assert_eq!(state, Vigil::Asking { since_ms: 200_000 });
        assert_eq!(act, Act::Ask);
    }

    /// Raised once, not once per tick. The poll runs every 1200ms and an `Ask`
    /// on each of them would be a notification storm rather than a question.
    #[test]
    fn the_question_is_raised_once() {
        let (state, _) = advance(Vigil::Listening, ASK_AFTER_MS, 200_000, false, false);
        for tick in 1..=5 {
            let (next, act) = advance(
                state,
                ASK_AFTER_MS + tick * 1_200,
                200_000 + tick * 1_200,
                false,
                false,
            );
            assert_eq!(next, state, "the question moved");
            assert_eq!(act, Act::Nothing, "asked again on tick {tick}");
        }
    }

    /// Speaking IS the answer. Somebody who comes back and talks must not also
    /// have to find a button — the question is a check that they are there, not
    /// a countdown to beat.
    #[test]
    fn speech_answers_the_question_without_a_click() {
        let asking = Vigil::Asking { since_ms: 200_000 };
        let (state, act) = advance(asking, 0, 260_000, true, false);
        assert_eq!(state, Vigil::Listening);
        assert_eq!(act, Act::Dismiss);
    }

    #[test]
    fn speech_while_listening_changes_nothing() {
        let (state, act) = advance(Vigil::Listening, 0, 5_000, true, false);
        assert_eq!(state, Vigil::Listening);
        assert_eq!(act, Act::Nothing);
    }

    #[test]
    fn an_unanswered_question_stops_the_recording() {
        let asking = Vigil::Asking { since_ms: 200_000 };
        let (_, act) = advance(
            asking,
            ASK_AFTER_MS,
            200_000 + ANSWER_WINDOW_MS,
            false,
            false,
        );
        assert_eq!(act, Act::Stop);
    }

    #[test]
    fn the_window_has_to_elapse_in_full() {
        let asking = Vigil::Asking { since_ms: 200_000 };
        let (state, act) = advance(
            asking,
            ASK_AFTER_MS,
            200_000 + ANSWER_WINDOW_MS - 1,
            false,
            false,
        );
        assert_eq!(state, asking);
        assert_eq!(act, Act::Nothing);
    }

    /// The recorder's clock does not advance while paused, so a pause during the
    /// question can leave `now` behind `since`. Wrapping there would read as an
    /// enormous elapsed time and stop the recording on the spot.
    #[test]
    fn a_clock_that_went_backwards_does_not_stop_anything() {
        let asking = Vigil::Asking { since_ms: 200_000 };
        let (state, act) = advance(asking, ASK_AFTER_MS, 100_000, false, false);
        assert_eq!(state, asking);
        assert_eq!(act, Act::Nothing);
    }

    /// Answering has to actually clear it, or the next tick stops the recording
    /// the user just said to keep.
    #[test]
    fn answering_clears_the_question() {
        let (state, act) = advance(answered(), ASK_AFTER_MS - 1, 400_000, false, false);
        assert_eq!(state, Vigil::Listening);
        assert_eq!(act, Act::Nothing);
    }

    /// Ending a recording over audio nobody has read is the one outcome this
    /// whole feature must not produce — somebody speaking in the last seconds
    /// of the window, or an utterance a failing provider handed back.
    #[test]
    fn unread_audio_holds_the_stop_back() {
        let asking = Vigil::Asking { since_ms: 200_000 };
        let (state, act) = advance(
            asking,
            ASK_AFTER_MS,
            200_000 + ANSWER_WINDOW_MS,
            false,
            true,
        );
        assert_eq!(state, asking, "the question was taken down");
        assert_eq!(act, Act::Nothing);
    }

    /// Grace, not veto. A room with a fan in it is above the segmenter's
    /// threshold forever, and a veto there would mean the stop never comes.
    #[test]
    fn audio_that_never_becomes_words_stops_it_anyway() {
        let asking = Vigil::Asking { since_ms: 200_000 };
        let (_, act) = advance(
            asking,
            ASK_AFTER_MS,
            200_000 + ANSWER_WINDOW_MS + UNREAD_GRACE_MS,
            false,
            true,
        );
        assert_eq!(act, Act::Stop);
    }
}
