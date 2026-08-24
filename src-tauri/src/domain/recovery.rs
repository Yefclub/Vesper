//! What to do about a meeting the application was recording when it stopped
//! running, and how to make the file it left behind readable.

use crate::domain::job::{MeetingRecord, MeetingStatus};

/// Whether this meeting is one nothing is left running to finish.
///
/// `Recording` and `Paused` are the two states only an interrupted run can
/// leave in the database: every ordinary way out of them goes through the stop
/// command, which moves the row on to `Transcribing` before it answers. A row
/// still in one of them at launch is therefore a meeting whose audio is on disk
/// with nobody coming back for it.
///
/// The meeting being recorded *right now* is the one exception, and it is why
/// `active` is a parameter rather than an assumption about when this is called:
/// its row says `Recording` because it is, and offering to recover a meeting
/// somebody is still speaking into would be an invitation to destroy it.
pub fn is_interrupted(meeting: &MeetingRecord, active: Option<&str>) -> bool {
    matches!(
        meeting.status,
        MeetingStatus::Recording | MeetingStatus::Paused
    ) && active != Some(meeting.id.as_str())
}

/// The meetings to offer recovery for, newest first as they were given.
pub fn interrupted(meetings: Vec<MeetingRecord>, active: Option<&str>) -> Vec<MeetingRecord> {
    meetings
        .into_iter()
        .filter(|m| is_interrupted(m, active))
        .collect()
}

/// The offset of the RIFF chunk's size field, which the format fixes at 4.
pub const RIFF_SIZE_OFFSET: u64 = 4;

/// What a WAV's two length fields should say for the bytes that are actually
/// in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderFix {
    pub riff_size: u32,
    pub data_size: u32,
    /// Where the `data` chunk's size field is. Not a constant like the RIFF
    /// one: anything may sit between `fmt ` and `data`, so it has to be found.
    pub data_size_offset: u64,
}

/// The correction a WAV header needs, or `None` when it needs none.
///
/// `hound` rewrites the length in the header on every flush, so a file the
/// process was killed part-way through declares fewer samples than it holds —
/// up to one flush interval of real speech, and the whole recording when the
/// kill beat the first flush. Nothing but the file's own size can recover that.
///
/// The opposite case is repaired too, and matters more than it looks: a header
/// declaring more than the file holds makes every reader fail at end-of-file
/// rather than return the audio that is there, so a recording cut short by a
/// full disk would be unreadable in its entirety.
///
/// Pure, and takes only the head of the file: the caller has a 500 MB recording
/// open and the answer is decided by its first few dozen bytes.
pub fn wav_header_fix(head: &[u8], file_len: u64) -> Option<HeaderFix> {
    if head.get(0..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    // Chunks start after the 12-byte `RIFF<size>WAVE` preamble, each one an id,
    // a length, and a body padded to an even number of bytes.
    let mut at = 12usize;
    let mut block_align: Option<u16> = None;
    loop {
        let id = head.get(at..at + 4)?;
        let size = le_u32(head, at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            // Bytes 12..14 of the `fmt ` body, per the format.
            block_align = le_u16(head, body + 12);
        } else if id == b"data" {
            // Refused rather than guessed. Without the frame size there is no
            // way to tell a whole frame from a half-written one, and a header
            // pointing at half a frame is a reader landing one channel out for
            // the rest of the file.
            let block = block_align.filter(|b| *b > 0)? as u64;
            let start = body as u64;
            let available = file_len.checked_sub(start)?;
            // A kill can land in the middle of a frame. What is left of it is
            // not audio, and counting it would swap the two channels over from
            // that point on.
            let data_size = available - available % block;
            if data_size == size as u64 {
                return None;
            }
            let data_size = u32::try_from(data_size).ok()?;
            // Everything after the eight bytes this field is itself part of.
            let riff_size = u32::try_from(start + data_size as u64 - 8).ok()?;
            return Some(HeaderFix {
                riff_size,
                data_size,
                data_size_offset: at as u64 + 4,
            });
        }
        // Strictly increasing, so a header this cannot make sense of runs off
        // the end of `head` and gives up rather than spinning.
        at = body.checked_add(size)?.checked_add(size & 1)?;
    }
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let s = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let s = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::MeetingEvent;

    fn meeting(id: &str, status: MeetingStatus) -> MeetingRecord {
        MeetingRecord {
            id: id.into(),
            title: "Q3 roadmap".into(),
            status,
            created_at: "2026-08-05T10:00:00Z".into(),
            updated_at: "2026-08-05T10:00:00Z".into(),
            duration_ms: 0,
            audio_path: Some("m.wav".into()),
            transcript_text: String::new(),
            summary: None,
            action_items: None,
            key_points: None,
            sections: Vec::new(),
            project: None,
            title_locked: false,
            cost_nano_usd: None,
            cost_label: None,
            speaker_me: None,
            speaker_others: None,
            brief: None,
        }
    }

    #[test]
    fn only_a_meeting_left_mid_recording_is_offered() {
        let all = vec![
            meeting("a", MeetingStatus::Recording),
            meeting("b", MeetingStatus::Paused),
            meeting("c", MeetingStatus::Ready),
            meeting("d", MeetingStatus::Transcribing),
            meeting("e", MeetingStatus::Failed),
        ];
        let ids: Vec<String> = interrupted(all, None).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    /// The one row that says `Recording` and means it.
    #[test]
    fn the_recording_in_progress_is_not_an_orphan() {
        let all = vec![
            meeting("a", MeetingStatus::Recording),
            meeting("b", MeetingStatus::Recording),
        ];
        let ids: Vec<String> = interrupted(all, Some("b"))
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["a"]);
    }

    /// Recovery is the stop that never ran, so it takes the same transition —
    /// and the walk from there is the one every recording takes.
    #[test]
    fn a_recovered_meeting_reaches_ready() {
        for from in [MeetingStatus::Recording, MeetingStatus::Paused] {
            let s = from.transition(MeetingEvent::StopRecording).unwrap();
            assert_eq!(s, MeetingStatus::Transcribing);
            assert_eq!(
                s.transition(MeetingEvent::TranscribeDone).unwrap(),
                MeetingStatus::Ready
            );
            // And Ready is where the summary is reachable from, which is the
            // other half of finishing a meeting.
            assert_eq!(
                MeetingStatus::Ready
                    .transition(MeetingEvent::StartSummarize)
                    .unwrap(),
                MeetingStatus::Summarizing
            );
        }
    }

    /// A canonical 44-byte header, as `hound` writes one, with the two length
    /// fields set to whatever the caller wants to have found on disk.
    fn header(declared_data: u32, block_align: u16) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + declared_data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // PCM
        h.extend_from_slice(&2u16.to_le_bytes()); // channels
        h.extend_from_slice(&16_000u32.to_le_bytes());
        h.extend_from_slice(&64_000u32.to_le_bytes());
        h.extend_from_slice(&block_align.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&declared_data.to_le_bytes());
        h
    }

    #[test]
    fn a_finished_file_needs_no_repair() {
        let h = header(800, 4);
        assert_eq!(wav_header_fix(&h, 44 + 800), None);
    }

    /// The case the whole feature exists for: the header was last written a
    /// flush ago and the audio since then is on disk with nothing pointing at
    /// it.
    #[test]
    fn a_header_left_behind_by_a_kill_is_brought_up_to_the_file() {
        let h = header(800, 4);
        let fix = wav_header_fix(&h, 44 + 4_800).unwrap();
        assert_eq!(fix.data_size, 4_800);
        assert_eq!(fix.riff_size, 36 + 4_800);
        // Byte 40 in a canonical header, found rather than assumed.
        assert_eq!(fix.data_size_offset, 40);
    }

    /// Nothing was ever flushed: the header says zero and the whole recording
    /// is behind it.
    #[test]
    fn a_header_that_was_never_updated_still_finds_the_audio() {
        let h = header(0, 4);
        let fix = wav_header_fix(&h, 44 + 1_600).unwrap();
        assert_eq!(fix.data_size, 1_600);
    }

    /// A kill can land between the two samples of a stereo frame. Half a frame
    /// is not audio, and keeping it would put Me in the Others channel for the
    /// rest of the file.
    #[test]
    fn a_half_written_frame_is_left_out() {
        let h = header(0, 4);
        let fix = wav_header_fix(&h, 44 + 1_602).unwrap();
        assert_eq!(fix.data_size, 1_600);
        assert_eq!(fix.riff_size, 36 + 1_600);
    }

    /// The other direction, which is the one that makes a file unreadable
    /// rather than merely short: every reader runs off the end.
    #[test]
    fn a_header_promising_more_than_the_file_holds_is_cut_back() {
        let h = header(9_999, 4);
        let fix = wav_header_fix(&h, 44 + 1_600).unwrap();
        assert_eq!(fix.data_size, 1_600);
    }

    /// A chunk between `fmt ` and `data` moves the field, so it is walked to
    /// rather than assumed at byte 40.
    #[test]
    fn the_data_length_is_found_past_an_intervening_chunk() {
        let mut h = header(0, 4);
        // Splice a LIST chunk of 6 bytes — odd body, so it carries a pad byte —
        // in front of the `data` header the builder put last.
        let data_header = h.split_off(36);
        h.extend_from_slice(b"LIST");
        h.extend_from_slice(&5u32.to_le_bytes());
        h.extend_from_slice(&[0u8; 6]);
        let spliced = h.len() as u64;
        h.extend_from_slice(&data_header);
        let fix = wav_header_fix(&h, spliced + 8 + 400).unwrap();
        assert_eq!(fix.data_size, 400);
        assert_eq!(fix.data_size_offset, spliced + 4);
    }

    #[test]
    fn something_that_is_not_a_wav_is_left_alone() {
        assert_eq!(wav_header_fix(b"not a riff file at all", 200), None);
        assert_eq!(wav_header_fix(&[], 200), None);
        // Truncated mid-header: there is no `data` chunk to be found, and
        // inventing one would write a length into whatever is at that offset.
        assert_eq!(wav_header_fix(&header(0, 4)[..20], 4_000), None);
        // A frame size of zero cannot say where a frame ends.
        assert_eq!(wav_header_fix(&header(0, 0), 4_000), None);
    }
}
