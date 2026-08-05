//! Where to cut live audio so a chunk is an utterance rather than a tick of a
//! clock.
//!
//! The live pass used to drain the recorder every 1200ms and hand whisper
//! whatever had arrived. Nothing about that interval has anything to do with
//! speech, so it cut words in half and asked a model trained on 30-second
//! windows to make sense of 1.2 seconds with no context. What reached the
//! screen was "Eu conto de…", "abandonado.", "pelo Google." — one bubble per
//! tick, sentences split across three of them, and `[BLANK_AUDIO]` wherever a
//! tick happened to land on a pause.
//!
//! This holds audio back until a pause says the sentence is over, and only then
//! transcribes. Slower to appear, and right when it does.
//!
//! **Silence is measured per frame, never per sample.** Speech crosses zero
//! constantly, so a per-sample threshold finds "silence" inside every vowel and
//! would cut more often than the clock it replaces.

/// Whether a buffer holds a frame loud enough to be speech, by the same measure
/// the segmenter cuts on.
///
/// For audio that has not reached a segmenter yet. Loud is not the same as
/// spoken — a fan clears this — so the answer is only ever worth "do not call
/// this silence", never "somebody talked".
pub fn has_speech(samples: &[i16], sample_rate: u32) -> bool {
    let frame = ((sample_rate.max(1) as u64 * FRAME_MS) / 1000).max(1) as usize;
    samples
        .chunks_exact(frame)
        .any(|f| f.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) >= QUIET_PEAK as u16)
}

/// One stretch of speech, bounded by pauses, ready to transcribe.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub pcm: Vec<i16>,
    /// Milliseconds from the start of the stream to this utterance's first
    /// sample. Counted from samples consumed, not from a wall clock: the two
    /// disagree whenever a device under-runs, and the transcript's timestamps
    /// have to match the audio rather than the machine.
    pub start_ms: u64,
    /// How far the segmenter moved past this utterance's start, in samples —
    /// the audio above plus the pause dropped behind it.
    ///
    /// Private, and only `put_back` reads it. Without it a retry has the pad and
    /// nothing else to rebuild the boundary with, so the next sentence runs into
    /// this one and every timestamp after it shifts earlier by the pause.
    advance: usize,
}

/// The window silence is judged in. Short enough to find the gap between two
/// sentences, long enough to contain the zero crossings inside one vowel.
const FRAME_MS: u64 = 20;

/// Peak amplitude below which a frame counts as silence.
///
/// Deliberately low. Set too high, speech is mistaken for a pause and the cut
/// lands mid-word — the bug this module exists to fix. Set too low, no pause is
/// ever found and every utterance ends at `MAX_MS`, which is merely a longer
/// chunk. The two failure modes are not symmetric, so this errs downward.
const QUIET_PEAK: i16 = 500;

/// How much quiet ends an utterance. Below this it is the gap between two words
/// rather than the end of a sentence.
const MIN_SILENCE_MS: u64 = 400;

/// How much speech an utterance needs before a pause may end it. A shorter run
/// is joined to what follows instead of becoming a bubble of its own — "Bye."
/// on its own line is the old behaviour, not a transcript.
const MIN_SPEECH_MS: u64 = 1_200;

/// The cut that happens whether or not anyone pauses.
///
/// Well inside whisper's 30-second window: the point is that a monologue still
/// reaches the screen while it is being spoken, not that the buffer never
/// overflows.
const MAX_MS: u64 = 15_000;

/// Silence kept after the last speech frame, so the final consonant is not
/// clipped off by the cut.
const PAD_MS: u64 = 200;

/// One channel's worth. Two speakers means two of these — they pause in
/// different places, and a boundary found in one is meaningless in the other.
pub struct Segmenter {
    sample_rate: u32,
    /// Held back, waiting for a boundary.
    pending: Vec<i16>,
    /// Samples consumed before `pending[0]`, since the start of the stream.
    consumed: u64,
    /// A boundary was declared from outside and not acted on yet.
    sealed: bool,
}

impl Segmenter {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            pending: Vec::new(),
            consumed: 0,
            sealed: false,
        }
    }

    /// Declare a boundary here without emitting anything yet.
    ///
    /// For pause. Somebody pressing it has finished their sentence as surely as
    /// a pause in the room, and what they say after resuming — which may be
    /// minutes later — is not the same utterance. It cannot emit on the spot:
    /// `pause_recording` is a synchronous command and transcription is not, so
    /// the boundary is remembered and the next `push` acts on it.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// The recorder's sample rate can only be known once capture is open, and a
    /// segmenter built before that would count its milliseconds against the
    /// wrong denominator.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let sample_rate = sample_rate.max(1);
        if sample_rate != self.sample_rate {
            // Everything held was measured against the old rate. Dropping it is
            // the honest move — a rate change mid-stream means the device was
            // replaced, and the samples either side are not one utterance.
            self.pending.clear();
            self.sample_rate = sample_rate;
        }
    }

    fn samples_for(&self, ms: u64) -> usize {
        ((self.sample_rate as u64 * ms) / 1000) as usize
    }

    fn ms_for(&self, samples: u64) -> u64 {
        (samples * 1000) / self.sample_rate as u64
    }

    /// Feed newly captured samples, and take whatever became a whole utterance.
    ///
    /// A vector rather than an option: a slow poll can deliver several seconds
    /// at once, and there can be two pauses inside it.
    pub fn push(&mut self, samples: &[i16]) -> Vec<Utterance> {
        let mut out = Vec::new();
        // Before the append, never after: the point of a seal is that what was
        // held and what arrives next are two different utterances.
        if std::mem::take(&mut self.sealed) {
            out.extend(self.flush());
        }
        self.pending.extend_from_slice(samples);
        while let Some(u) = self.take_one() {
            out.push(u);
        }
        out
    }

    /// Everything still held, whether or not a pause has arrived.
    ///
    /// For stop and pause: the last sentence of a meeting is not followed by
    /// silence, it is followed by the user pressing a button, and waiting for a
    /// boundary that will never come would drop it.
    pub fn flush(&mut self) -> Option<Utterance> {
        let all = self.pending.len();
        if all == 0 {
            return None;
        }
        let frames = self.frames();
        let frame = self.samples_for(FRAME_MS).max(1);
        // A tail shorter than one frame has no frame to judge, so it is taken
        // whole rather than dropped — this is the end of a meeting, and 20ms of
        // somebody's last word is still their last word.
        let (start, end) = match frames.iter().position(|quiet| !quiet) {
            Some(first) => {
                let last = frames
                    .iter()
                    .rposition(|quiet| !quiet)
                    .expect("a speech frame was just found");
                (
                    first * frame,
                    ((last + 1) * frame + self.samples_for(PAD_MS)).min(all),
                )
            }
            None if frames.is_empty() => (0, all),
            // Whole frames, all silent. Whatever sits in the partial tail is
            // shorter than 20ms and not worth a model call.
            None => return self.drop_all(),
        };
        Some(self.cut(start, end, all))
    }

    /// Consume everything and hand back nothing.
    fn drop_all(&mut self) -> Option<Utterance> {
        self.consumed += self.pending.len() as u64;
        self.pending.clear();
        None
    }

    /// Put an utterance back at the head, unconsumed.
    ///
    /// The transcription of it failed, and audio dropped here is a slice of a
    /// meeting nobody can get back — the same reason the recorder rewinds its
    /// own cursor when a provider rejects a chunk.
    pub fn put_back(&mut self, u: Utterance) {
        // Wound back to where this utterance began, not by its length: the cut
        // that produced it also dropped the pause behind it, so subtracting the
        // audio alone would leave the clock ahead of the buffer and the retry
        // would land at a timestamp the words were never spoken at.
        self.consumed = (u.start_ms * self.sample_rate as u64) / 1000;
        let silence = u.advance.saturating_sub(u.pcm.len());
        let mut head = u.pcm;
        // Zeros for the pause that was dropped. Synthetic, and honest: this
        // module's own measure is what called it silence in the first place, and
        // the boundary it rebuilds is the reason the retry cuts where the first
        // attempt did.
        head.resize(head.len() + silence, 0);
        head.append(&mut self.pending);
        self.pending = head;
    }

    /// Whether an utterance is under way — audio held back, waiting for the
    /// pause that ends it.
    ///
    /// True means somebody is speaking right now, or spoke and the words have
    /// not come back from a model yet. `take_one` drops leading silence, so
    /// what is held always begins at speech and a quiet room holds nothing.
    pub fn holding(&self) -> bool {
        !self.pending.is_empty()
    }

    /// One frame per `FRAME_MS`, true where the frame is silent. The trailing
    /// partial frame is not judged — it is not a whole window yet, and calling
    /// it silent would end an utterance on the poll interval again.
    fn frames(&self) -> Vec<bool> {
        let frame = self.samples_for(FRAME_MS).max(1);
        self.pending
            .chunks_exact(frame)
            .map(|f| f.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) < QUIET_PEAK as u16)
            .collect()
    }

    /// `pending[start..end]` as an utterance, with everything up to `drop`
    /// removed from the buffer and charged to the clock.
    fn cut(&mut self, start: usize, end: usize, drop: usize) -> Utterance {
        let u = Utterance {
            pcm: self.pending[start..end].to_vec(),
            start_ms: self.ms_for(self.consumed + start as u64),
            // From this utterance's own start, not from the head of the buffer:
            // `flush` trims leading silence that belongs to nobody.
            advance: drop.saturating_sub(start),
        };
        self.consumed += drop as u64;
        self.pending.drain(..drop);
        u
    }

    /// The next boundary in the buffer, if there is one yet.
    fn take_one(&mut self) -> Option<Utterance> {
        let frames = self.frames();
        if frames.is_empty() {
            return None;
        }
        let frame = self.samples_for(FRAME_MS).max(1);

        // Nobody has spoken. Dropped rather than held: silence never needs
        // transcribing, and holding it would let a quiet meeting reach `MAX_MS`
        // and send fifteen seconds of room tone to the model — which is where
        // `[BLANK_AUDIO]` came from.
        let Some(first_speech) = frames.iter().position(|quiet| !quiet) else {
            let whole = frames.len() * frame;
            self.consumed += whole as u64;
            self.pending.drain(..whole);
            return None;
        };

        // Leading silence is not part of the sentence and not worth the model's
        // time. Charged to the clock so the utterance's own timestamp stays put.
        if first_speech > 0 {
            let lead = first_speech * frame;
            self.consumed += lead as u64;
            self.pending.drain(..lead);
            return self.take_one();
        }

        // The FIRST pause long enough to end a sentence, not the last speech in
        // the buffer. A slow poll can carry two sentences and the pause between
        // them, and looking only at where speech stops last makes those one
        // utterance with the pause buried inside it.
        let min_silence = (MIN_SILENCE_MS / FRAME_MS) as usize;
        let min_speech = (MIN_SPEECH_MS / FRAME_MS) as usize;
        // Only a pause that starts inside the ceiling can be a boundary. One
        // drain can carry far more than `MAX_MS` — the ticker waits behind a
        // transcription that is still running, and the audio piles up meanwhile
        // — so a pause found at second forty would otherwise produce a
        // forty-second utterance out of a fifteen-second bound, past the window
        // whisper was trained on. Its length is measured past the limit, though:
        // where a pause ends is not a boundary, only where it begins.
        let limit = frames.len().min((MAX_MS / FRAME_MS) as usize);
        let mut i = 0;
        while i < limit {
            if !frames[i] {
                i += 1;
                continue;
            }
            let gap = i;
            while i < frames.len() && frames[i] {
                i += 1;
            }
            // Voiced frames only, not `gap` itself: the frames before this pause
            // include any shorter breaths that were kept inside the utterance,
            // and counting those lets a second of speech with two gaps in it
            // clear a threshold meant to hold exactly that back.
            let voiced = frames[..gap].iter().filter(|quiet| !**quiet).count();
            if i - gap >= min_silence && voiced >= min_speech {
                // Cut after the speech plus a pad, and drop the whole pause —
                // what follows it is the next sentence, and it starts at speech.
                // The utterance carries how far this moved the buffer, so a
                // failed transcription can put the pause back with it.
                let end = (gap * frame + self.samples_for(PAD_MS)).min(self.pending.len());
                let drop = (i * frame).min(self.pending.len());
                return Some(self.cut(0, end, drop));
            }
        }

        if self.ms_for(self.pending.len() as u64) >= MAX_MS {
            // Exactly the ceiling, not the whole buffer. `push` loops, so a
            // backlog comes out as several utterances of this size rather than
            // one enormous one.
            let end = self.samples_for(MAX_MS).min(self.pending.len());
            return Some(self.cut(0, end, end));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    fn quiet(ms: u64) -> Vec<i16> {
        vec![0; (SR as u64 * ms / 1000) as usize]
    }

    /// Something a frame's peak reads as speech, oscillating through zero the
    /// way real audio does — which is the whole reason silence is judged per
    /// frame and not per sample.
    fn speech(ms: u64) -> Vec<i16> {
        let n = (SR as u64 * ms / 1000) as usize;
        (0..n)
            .map(|i| if i % 2 == 0 { 8_000 } else { -8_000 })
            .collect()
    }

    #[test]
    fn a_short_burst_is_held_rather_than_emitted() {
        let mut s = Segmenter::new(SR);
        // Exactly the old behaviour's chunk: 1.2s arriving on the poll.
        assert!(s.push(&speech(600)).is_empty());
        assert!(
            s.push(&quiet(600)).is_empty(),
            "600ms of speech is a fragment"
        );
    }

    #[test]
    fn a_pause_after_enough_speech_ends_the_utterance() {
        let mut s = Segmenter::new(SR);
        assert!(s.push(&speech(2_000)).is_empty());
        let out = s.push(&quiet(500));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_ms, 0);
        // The speech, plus the pad, and none of the rest of the pause.
        let ms = out[0].pcm.len() as u64 * 1000 / SR as u64;
        assert!((2_150..=2_250).contains(&ms), "{ms}ms");
    }

    #[test]
    fn a_gap_between_words_does_not_cut() {
        let mut s = Segmenter::new(SR);
        s.push(&speech(1_500));
        // 200ms is a breath, not the end of a sentence.
        assert!(s.push(&quiet(200)).is_empty());
        assert!(s.push(&speech(800)).is_empty());
        let out = s.push(&quiet(500));
        assert_eq!(out.len(), 1, "the two runs are one utterance");
        let ms = out[0].pcm.len() as u64 * 1000 / SR as u64;
        assert!(ms > 2_400, "the gap was kept inside the utterance: {ms}ms");
    }

    #[test]
    fn silence_alone_never_reaches_the_model() {
        let mut s = Segmenter::new(SR);
        for _ in 0..30 {
            assert!(s.push(&quiet(1_000)).is_empty());
        }
        // And it did not pile up waiting for a boundary either.
        assert!(s.pending.is_empty());
    }

    /// The silence guard asks this before it ends a recording, so a quiet room
    /// has to answer "nothing held" and a sentence still being spoken has to
    /// answer "wait".
    #[test]
    fn holding_says_whether_a_sentence_is_under_way() {
        let mut s = Segmenter::new(SR);
        assert!(!s.holding(), "a fresh segmenter holds nothing");
        s.push(&quiet(3_000));
        assert!(!s.holding(), "a quiet room held audio");
        s.push(&speech(2_000));
        assert!(s.holding(), "speech without its pause was not held");
        s.push(&quiet(1_000));
        assert!(
            !s.holding(),
            "audio stayed behind after the utterance was taken"
        );
    }

    #[test]
    fn a_monologue_is_cut_at_the_ceiling() {
        let mut s = Segmenter::new(SR);
        let mut out = Vec::new();
        for _ in 0..20 {
            out.extend(s.push(&speech(1_000)));
        }
        assert_eq!(
            out.len(),
            1,
            "one cut at the ceiling, not none and not four"
        );
        let ms = out[0].pcm.len() as u64 * 1000 / SR as u64;
        assert!((15_000..16_000).contains(&ms), "{ms}ms");
    }

    /// A poll delayed behind a running transcription hands over a backlog. It
    /// comes out as utterances of the ceiling, never as one of forty seconds —
    /// whisper was trained on a thirty-second window.
    #[test]
    fn a_backlog_is_cut_into_ceilings_not_handed_over_whole() {
        let mut s = Segmenter::new(SR);
        let out = s.push(&speech(40_000));
        assert_eq!(out.len(), 2);
        for u in &out {
            let ms = u.pcm.len() as u64 * 1000 / SR as u64;
            assert_eq!(ms, 15_000, "an utterance past the ceiling");
        }
        assert_eq!(out[1].start_ms, 15_000);
    }

    /// And a pause beyond the ceiling does not get to be the boundary either.
    #[test]
    fn a_pause_past_the_ceiling_does_not_widen_the_cut() {
        let mut s = Segmenter::new(SR);
        let mut audio = speech(20_000);
        audio.extend(quiet(600));
        let out = s.push(&audio);
        let ms = out[0].pcm.len() as u64 * 1000 / SR as u64;
        assert_eq!(ms, 15_000, "the pause at 20s widened the first utterance");
    }

    #[test]
    fn timestamps_follow_the_audio_not_the_call() {
        let mut s = Segmenter::new(SR);
        s.push(&quiet(3_000));
        s.push(&speech(2_000));
        let out = s.push(&quiet(500));
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].start_ms, 3_000,
            "the leading silence still costs time"
        );
    }

    #[test]
    fn two_pauses_in_one_poll_give_two_utterances() {
        let mut s = Segmenter::new(SR);
        let mut audio = speech(1_500);
        audio.extend(quiet(600));
        audio.extend(speech(1_500));
        audio.extend(quiet(600));
        let out = s.push(&audio);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_ms, 0);
        assert!(out[1].start_ms >= 2_100, "{}", out[1].start_ms);
    }

    /// A second of speech with breaths in it is still a second of speech. The
    /// threshold counts what was said, not how long the buffer has been open.
    #[test]
    fn breaths_do_not_count_towards_the_minimum() {
        let mut s = Segmenter::new(SR);
        let mut audio = speech(1_000);
        audio.extend(quiet(300));
        audio.extend(speech(100));
        audio.extend(quiet(500));
        // 1100ms voiced across 1400ms of buffer: held, not emitted.
        assert!(s.push(&audio).is_empty());
        // And once the missing 100ms arrives, it goes.
        let mut more = speech(200);
        more.extend(quiet(500));
        assert_eq!(s.push(&more).len(), 1);
    }

    /// The pause behind an utterance is not thrown away at the cut, so a failed
    /// transcription has the whole boundary to put back.
    #[test]
    fn a_retry_after_failure_keeps_the_boundary() {
        let mut s = Segmenter::new(SR);
        let mut audio = speech(2_000);
        audio.extend(quiet(600));
        audio.extend(speech(2_000));
        audio.extend(quiet(600));
        let out = s.push(&audio);
        assert_eq!(out.len(), 2);
        let (first, second) = (out[0].clone(), out[1].clone());

        // The transcription of the first one failed. Everything goes back, in
        // the order the live path puts it back.
        let mut s = Segmenter::new(SR);
        s.push(&audio);
        s.put_back(second.clone());
        s.put_back(first.clone());
        let again = s.push(&[]);
        assert_eq!(again.len(), 2, "the second boundary survived the retry");
        assert_eq!(again[0].start_ms, first.start_ms);
        assert_eq!(
            again[1].start_ms, second.start_ms,
            "the dropped pause was still charged to the clock"
        );
    }

    #[test]
    fn flush_gives_up_the_last_sentence() {
        let mut s = Segmenter::new(SR);
        s.push(&speech(500));
        let out = s.flush().expect("a stop must not drop the last words");
        let ms = out.pcm.len() as u64 * 1000 / SR as u64;
        assert!((480..=520).contains(&ms), "{ms}ms");
    }

    /// Pausing ends the sentence. What is said after a resume, which may be
    /// minutes later, is not a continuation of it.
    #[test]
    fn a_seal_keeps_what_follows_out_of_the_held_utterance() {
        let mut s = Segmenter::new(SR);
        assert!(s.push(&speech(800)).is_empty(), "held, too short so far");
        s.seal();
        let mut after = speech(2_000);
        after.extend(quiet(500));
        let out = s.push(&after);
        assert_eq!(out.len(), 2, "the sealed one, then the new one");
        let first = out[0].pcm.len() as u64 * 1000 / SR as u64;
        assert!((780..=820).contains(&first), "{first}ms");
        assert_eq!(out[0].start_ms, 0);
        assert_eq!(out[1].start_ms, 800);
    }

    #[test]
    fn flush_on_silence_gives_nothing() {
        let mut s = Segmenter::new(SR);
        s.push(&quiet(2_000));
        assert!(s.flush().is_none());
    }

    #[test]
    fn put_back_restores_the_audio_and_the_clock() {
        let mut s = Segmenter::new(SR);
        s.push(&speech(2_000));
        let out = s.push(&quiet(500));
        assert_eq!(out.len(), 1);
        let first = out.into_iter().next().unwrap();
        let at = first.start_ms;
        let len = first.pcm.len();
        s.put_back(first);
        // The audio, plus the pause the cut dropped behind it — 2500ms, not the
        // 2200ms of the utterance itself. Without the pause the retry would run
        // the next sentence into this one.
        assert!(s.pending.len() > len, "the boundary came back too");
        assert_eq!(s.pending.len(), (SR as usize * 2_500) / 1000);
        // And the next boundary hands out the same slice at the same offset.
        let again = s.push(&[]);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].start_ms, at);
        assert_eq!(again[0].pcm.len(), len, "and the same slice");
    }
}
