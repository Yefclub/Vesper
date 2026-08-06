//! Dual-channel audio capture: microphone (Me) + system loopback (Others).
//! Uses flexaudio backends: WASAPI loopback (Windows), CoreAudio taps (macOS),
//! PipeWire (Linux) for system audio — never fakes Others from the mic.

use crate::audio::levels::ChannelLevels;
use crate::domain::channels::ChannelSelection;
use crate::domain::segmenter::has_speech;
use crate::domain::speaker::merge_dual_channel;
use flexaudio::{open, OutputFormat, SourceKind, StreamConfig};
use hound::{WavSpec, WavWriter};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("audio device error: {0}")]
    Device(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not recording")]
    NotRecording,
    #[error("already recording")]
    AlreadyRecording,
}

struct RecorderInner {
    /// Full meeting capture (never cleared by live STT).
    mic_samples: Vec<i16>,
    sys_samples: Vec<i16>,
    /// Cursor for live-STT windows — advances on drain without destroying recording.
    mic_stt_pos: usize,
    sys_stt_pos: usize,
    sample_rate: u32,
    levels: ChannelLevels,
    out_path: Option<PathBuf>,
    start: Option<Instant>,
    elapsed_before_pause_ms: u64,
    /// True when a real system-loopback stream is active.
    system_loopback_active: bool,
    /// The channels this recording was started with.
    ///
    /// Kept here rather than read back from settings, because the two stop
    /// agreeing the moment a recording is running: `start` copies the selection
    /// into the streams, so a switch flipped afterwards belongs to the next
    /// recording. A meter reading zero has two meanings — nobody is talking,
    /// or nothing is listening — and this is what tells them apart.
    channels: ChannelSelection,
}

pub struct DualChannelRecorder {
    inner: Arc<Mutex<RecorderInner>>,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl DualChannelRecorder {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecorderInner {
                mic_samples: Vec::new(),
                sys_samples: Vec::new(),
                mic_stt_pos: 0,
                sys_stt_pos: 0,
                sample_rate: 16_000,
                levels: ChannelLevels::default(),
                out_path: None,
                start: None,
                elapsed_before_pause_ms: 0,
                system_loopback_active: false,
                channels: ChannelSelection::default(),
            })),
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            workers: Mutex::new(Vec::new()),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn levels(&self) -> ChannelLevels {
        self.inner.lock().levels
    }

    pub fn system_loopback_active(&self) -> bool {
        self.inner.lock().system_loopback_active
    }

    /// The channels the running capture is listening to. After a stop it is
    /// whatever the last `start` was given, the same way `elapsed_ms` keeps the
    /// last recording's clock.
    pub fn channels(&self) -> ChannelSelection {
        self.inner.lock().channels
    }

    pub fn elapsed_ms(&self) -> u64 {
        let g = self.inner.lock();
        elapsed(
            g.start,
            g.elapsed_before_pause_ms,
            self.paused.load(Ordering::SeqCst),
            Instant::now(),
        )
    }

    pub fn start(
        &self,
        out_path: PathBuf,
        channels: ChannelSelection,
        mic_device_id: Option<String>,
        system_device_id: Option<String>,
    ) -> Result<(), CaptureError> {
        if self.running.load(Ordering::SeqCst) {
            return Err(CaptureError::AlreadyRecording);
        }

        let sample_rate = 16_000u32;
        {
            let mut g = self.inner.lock();
            g.mic_samples.clear();
            g.sys_samples.clear();
            g.mic_stt_pos = 0;
            g.sys_stt_pos = 0;
            g.sample_rate = sample_rate;
            g.out_path = Some(out_path);
            g.start = Some(Instant::now());
            g.elapsed_before_pause_ms = 0;
            g.levels = ChannelLevels::default();
            g.system_loopback_active = false;
            g.channels = channels;
        }

        self.stop_flag.store(false, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);

        // One handle per channel that is on, so a recording with a channel
        // switched off has one worker and one open stream. Nothing is opened
        // and thrown away: an open stream is a microphone light on the user's
        // machine and a device another application then cannot take exclusively
        // — a recorder told not to listen must not be holding the hardware.
        let mut workers: Vec<JoinHandle<()>> = Vec::new();
        // Each worker says once whether its stream opened, and `start` waits to
        // hear from every one it spawned before calling this a recording.
        //
        // Opening happens on the worker rather than here, so without this the
        // failure had nowhere to go: the thread printed and returned while this
        // marked the recorder running. With two channels that cost half a
        // meeting; with one selected it cost all of it, and the user found out
        // when they pressed Stop on a silent file.
        let (ready_tx, ready_rx) = mpsc::channel::<bool>();

        // —— Microphone (Me) ——
        if channels.me {
            let stop = self.stop_flag.clone();
            let paused = self.paused.clone();
            let inner_mic = self.inner.clone();
            let ready = ready_tx.clone();
            let mic_handle = thread::Builder::new()
                .name("vesper-mic".into())
                .spawn(move || {
                    let cfg = StreamConfig {
                        kind: SourceKind::Mic,
                        device_id: mic_device_id,
                        output: OutputFormat {
                            sample_rate,
                            channels: 1,
                        },
                        exclude_self: false,
                        ..Default::default()
                    };
                    let mut stream = match open(cfg) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("mic open failed: {e}");
                            let _ = ready.send(false);
                            return;
                        }
                    };
                    if let Err(e) = stream.start() {
                        eprintln!("mic start failed: {e}");
                        let _ = ready.send(false);
                        return;
                    }
                    let _ = ready.send(true);
                    // Dropped here rather than at the end of the thread, and
                    // load-bearing: a worker that dies without reporting has to
                    // show up as the channel closing. Held for the length of
                    // the capture, the surviving worker's copy would keep it
                    // open and `start` would wait for a message nobody is left
                    // to send.
                    drop(ready);
                    let mut stream_paused = false;
                    while !stop.load(Ordering::SeqCst) {
                        let want_pause = paused.load(Ordering::SeqCst);
                        match capture_step(want_pause, stream_paused) {
                            StreamAction::EnterPause => {
                                stream.pause();
                                stream_paused = true;
                                // Chunks already in the ring are pre-pause audio and
                                // belong in the recording, so take them now.
                                while let Some(chunk) = stream.poll_chunk() {
                                    let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                                    inner_mic.lock().mic_samples.extend_from_slice(&pcm);
                                }
                            }
                            StreamAction::LeavePause => {
                                stream.resume();
                                stream_paused = false;
                            }
                            StreamAction::StayPaused | StreamAction::Poll => {}
                        }
                        if want_pause {
                            // Nothing is arriving; a meter frozen at the last
                            // pre-pause value says the opposite.
                            {
                                let mut g = inner_mic.lock();
                                g.levels.me_peak = 0.0;
                                g.levels.me_rms = 0.0;
                            }
                            thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        while let Some(chunk) = stream.poll_chunk() {
                            let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                            let mut g = inner_mic.lock();
                            g.levels.me_peak = chunk.peak;
                            g.levels.me_rms = chunk.rms;
                            g.mic_samples.extend_from_slice(&pcm);
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    // Pause, then drain, then stop. Stop can arrive between a pause
                    // and the loop noticing it, leaving pre-pause chunks in the ring
                    // with nothing left to take them — real audio the user already
                    // recorded, and on a very short take all of it.
                    //
                    // `pause()` rather than draining straight away: it halts the
                    // producer while leaving the ring readable, which is the property
                    // `EnterPause` above is already built on. Draining first leaves a
                    // window for one more chunk to land behind the sweep, and
                    // `stop()` first would take the queue down with the producer.
                    stream.pause();
                    while let Some(chunk) = stream.poll_chunk() {
                        let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                        inner_mic.lock().mic_samples.extend_from_slice(&pcm);
                    }
                    stream.stop();
                    // And once more after the join. The intake worker can have read
                    // `paused == false` and enqueued one last chunk between the sweep
                    // above and the pause taking effect; `stop()` joins it without
                    // discarding the queue, so that chunk is still there to take.
                    // Costs nothing when there is nothing: `poll_chunk` answers None.
                    while let Some(chunk) = stream.poll_chunk() {
                        let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                        inner_mic.lock().mic_samples.extend_from_slice(&pcm);
                    }
                })
                .map_err(|e| CaptureError::Device(e.to_string()))?;
            workers.push(mic_handle);
        }

        // —— System loopback (Others) — real WASAPI/CoreAudio/PipeWire via flexaudio ——
        if channels.others {
            let stop_sys = self.stop_flag.clone();
            let paused_sys = self.paused.clone();
            let inner_sys = self.inner.clone();
            let ready = ready_tx.clone();
            let sys_handle = thread::Builder::new()
                .name("vesper-system".into())
                .spawn(move || {
                    let cfg = StreamConfig {
                        kind: SourceKind::SystemLoopback,
                        device_id: system_device_id,
                        output: OutputFormat {
                            sample_rate,
                            channels: 1,
                        },
                        // Avoid feedback of Vesper's own UI sounds into the mix.
                        exclude_self: true,
                        ..Default::default()
                    };
                    let mut stream = match open(cfg) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("system loopback open failed: {e}");
                            let _ = ready.send(false);
                            return;
                        }
                    };
                    if let Err(e) = stream.start() {
                        eprintln!("system loopback start failed: {e}");
                        let _ = ready.send(false);
                        return;
                    }
                    // See the microphone worker above for why the sender goes
                    // as soon as it has spoken.
                    let _ = ready.send(true);
                    drop(ready);
                    {
                        inner_sys.lock().system_loopback_active = true;
                    }
                    let mut stream_paused = false;
                    while !stop_sys.load(Ordering::SeqCst) {
                        let want_pause = paused_sys.load(Ordering::SeqCst);
                        match capture_step(want_pause, stream_paused) {
                            StreamAction::EnterPause => {
                                stream.pause();
                                stream_paused = true;
                                // Chunks already in the ring are pre-pause audio and
                                // belong in the recording, so take them now.
                                while let Some(chunk) = stream.poll_chunk() {
                                    let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                                    inner_sys.lock().sys_samples.extend_from_slice(&pcm);
                                }
                            }
                            StreamAction::LeavePause => {
                                stream.resume();
                                stream_paused = false;
                            }
                            StreamAction::StayPaused | StreamAction::Poll => {}
                        }
                        if want_pause {
                            // Nothing is arriving; a meter frozen at the last
                            // pre-pause value says the opposite.
                            {
                                let mut g = inner_sys.lock();
                                g.levels.others_peak = 0.0;
                                g.levels.others_rms = 0.0;
                            }
                            thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        while let Some(chunk) = stream.poll_chunk() {
                            let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                            let mut g = inner_sys.lock();
                            g.levels.others_peak = chunk.peak;
                            g.levels.others_rms = chunk.rms;
                            g.sys_samples.extend_from_slice(&pcm);
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    // Pause, then drain, then stop. Stop can arrive between a pause
                    // and the loop noticing it, leaving pre-pause chunks in the ring
                    // with nothing left to take them — real audio the user already
                    // recorded, and on a very short take all of it.
                    //
                    // `pause()` rather than draining straight away: it halts the
                    // producer while leaving the ring readable, which is the property
                    // `EnterPause` above is already built on. Draining first leaves a
                    // window for one more chunk to land behind the sweep, and
                    // `stop()` first would take the queue down with the producer.
                    stream.pause();
                    while let Some(chunk) = stream.poll_chunk() {
                        let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                        inner_sys.lock().sys_samples.extend_from_slice(&pcm);
                    }
                    stream.stop();
                    // And once more after the join — see the mic thread above.
                    while let Some(chunk) = stream.poll_chunk() {
                        let pcm = f32_to_i16_mono(&chunk.data, chunk.frames, 1);
                        inner_sys.lock().sys_samples.extend_from_slice(&pcm);
                    }
                    inner_sys.lock().system_loopback_active = false;
                })
                .map_err(|e| CaptureError::Device(e.to_string()))?;
            workers.push(sys_handle);
        }

        // This side's own sender goes before the wait, or a worker that died
        // without reporting would leave the channel open on a copy nobody is
        // holding and the loop below with nothing to end it.
        drop(ready_tx);
        if !streams_are_running(&ready_rx, workers.len(), STREAM_READY_TIMEOUT) {
            // Refused only when nothing opened, never when something did: a
            // two-channel meeting whose microphone failed still has the room,
            // and half a recording beats none. With one channel selected the
            // two are the same question — and answering it wrongly is a meeting
            // recorded to silence with nothing on screen having said so.
            //
            // Nothing to tear down: every worker that could not open its stream
            // has already returned, which is what it just reported.
            return Err(CaptureError::Device(
                "no capture device could be opened".into(),
            ));
        }

        *self.workers.lock() = workers;

        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn pause(&self) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }
        if !self.paused.swap(true, Ordering::SeqCst) {
            let mut g = self.inner.lock();
            if let Some(start) = g.start.take() {
                g.elapsed_before_pause_ms += start.elapsed().as_millis() as u64;
            }
        }
    }

    pub fn resume(&self) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }
        if self.paused.swap(false, Ordering::SeqCst) {
            self.inner.lock().start = Some(Instant::now());
        }
    }

    pub fn stop(&self) -> Result<PathBuf, CaptureError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(CaptureError::NotRecording);
        }
        self.running.store(false, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.stop_flag.store(true, Ordering::SeqCst);

        // Join capture workers
        let handles: Vec<_> = self.workers.lock().drain(..).collect();
        for h in handles {
            let _ = h.join();
        }

        let mut g = self.inner.lock();
        if let Some(start) = g.start.take() {
            g.elapsed_before_pause_ms += start.elapsed().as_millis() as u64;
        }
        let path = g
            .out_path
            .clone()
            .ok_or_else(|| CaptureError::Device("missing output path".into()))?;
        write_dual_wav(&path, g.sample_rate, &g.mic_samples, &g.sys_samples)?;
        Ok(path)
    }

    /// Return PCM **since the last drain** for live STT, without removing it from
    /// the full recording buffers used by `stop()` → `write_dual_wav`.
    pub fn drain_chunks(&self) -> (Vec<i16>, Vec<i16>, u32) {
        let mut g = self.inner.lock();
        let sample_rate = g.sample_rate;
        // Split borrows carefully: copy windows first, then advance cursors.
        let mic_pos = g.mic_stt_pos.min(g.mic_samples.len());
        let sys_pos = g.sys_stt_pos.min(g.sys_samples.len());
        let mic = g.mic_samples[mic_pos..].to_vec();
        let sys = g.sys_samples[sys_pos..].to_vec();
        g.mic_stt_pos = g.mic_samples.len();
        g.sys_stt_pos = g.sys_samples.len();
        (mic, sys, sample_rate)
    }

    /// Whether the audio no live pass has read yet is loud enough to be speech.
    ///
    /// Non-destructive, and that is the point: the silence guard has to know
    /// whether somebody started talking after the last drain, and a drain to
    /// find out would race the stop that takes the tail — under one lock, with
    /// the cursors left where they were, there is nothing to race.
    pub fn unread_has_speech(&self) -> bool {
        let g = self.inner.lock();
        let mic = &g.mic_samples[g.mic_stt_pos.min(g.mic_samples.len())..];
        let sys = &g.sys_samples[g.sys_stt_pos.min(g.sys_samples.len())..];
        has_speech(mic, g.sample_rate) || has_speech(sys, g.sample_rate)
    }

    /// Put back what `drain_chunks` just handed out.
    ///
    /// The samples never left `mic_samples`/`sys_samples` — draining only advances
    /// a cursor — so this rewinds the cursor by what was taken. A transcription
    /// that failed cost the meeting that slice of audio every time, which with a
    /// provider rejecting every chunk meant the live transcript was being shredded
    /// 1200ms at a time while the window showed nothing.
    ///
    /// Saturating on purpose: a rewind can only ever race a drain, and landing at
    /// zero re-reads audio that was already transcribed. Duplicated text is
    /// recoverable; a hole in the transcript is not.
    pub fn rewind_chunks(&self, mic_len: usize, sys_len: usize) {
        let mut g = self.inner.lock();
        g.mic_stt_pos = g.mic_stt_pos.saturating_sub(mic_len);
        g.sys_stt_pos = g.sys_stt_pos.saturating_sub(sys_len);
    }

    /// Snapshot of the full dual-channel recording (for tests / diagnostics).
    pub fn recording_len(&self) -> (usize, usize) {
        let g = self.inner.lock();
        (g.mic_samples.len(), g.sys_samples.len())
    }

    /// Test/helper: append PCM as if capture threads produced it.
    #[cfg(test)]
    pub fn push_samples_for_test(&self, mic: &[i16], sys: &[i16]) {
        let mut g = self.inner.lock();
        g.mic_samples.extend_from_slice(mic);
        g.sys_samples.extend_from_slice(sys);
    }

    /// Test/helper: write current full buffers to path (same as stop without joining streams).
    #[cfg(test)]
    pub fn flush_recording_for_test(&self, path: &Path) -> Result<(), CaptureError> {
        let g = self.inner.lock();
        write_dual_wav(path, g.sample_rate, &g.mic_samples, &g.sys_samples)
    }
}

/// What a capture worker owes the backend on this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamAction {
    /// The clock is running: take whatever the stream has.
    Poll,
    /// First tick of a pause. Stop the backend's delivery, then take what is
    /// already in the ring — that audio is from before the pause.
    EnterPause,
    /// Still paused. The backend is already stopped; touch nothing.
    StayPaused,
    /// First tick after a resume. Start delivery again before polling, or the
    /// first poll returns nothing and the next one returns a gap.
    LeavePause,
}

/// The pause edge, as a value a test can assert on.
///
/// Flipping the worker's own `continue` is not a pause: flexaudio keeps filling
/// a 50-chunk (1 s) DROP_OLDEST ring, and the first poll after a resume splices
/// that second of pause audio into the recording. Only the edges may talk to the
/// backend, so a stray extra `pause()` cannot discard a chunk mid-pause.
fn capture_step(want_pause: bool, was_paused: bool) -> StreamAction {
    match (want_pause, was_paused) {
        (true, false) => StreamAction::EnterPause,
        (true, true) => StreamAction::StayPaused,
        (false, true) => StreamAction::LeavePause,
        (false, false) => StreamAction::Poll,
    }
}

/// The recording clock at `now`: everything before the current pause, plus the
/// running span when the clock is not paused.
///
/// Lifted out of `elapsed_ms` so the paused case is reachable without an audio
/// device — the WAV's sample count and this number have to agree, and only one
/// of the two can be tested here.
fn elapsed(start: Option<Instant>, before_ms: u64, paused: bool, now: Instant) -> u64 {
    match start {
        Some(start) if !paused => before_ms + now.duration_since(start).as_millis() as u64,
        _ => before_ms,
    }
}

/// How long `start` waits to hear that its streams are open.
///
/// Slack rather than a budget: opening a capture stream is tens of
/// milliseconds, and a device that is going to refuse refuses about as quickly.
/// It is bounded at all because a wedged driver can block inside the backend
/// without ever returning or erroring, and this wait runs on the thread that
/// answers the Record button.
const STREAM_READY_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether `start` may call the capture running: did any of the `expected`
/// workers report a stream it had opened.
///
/// Lifted out of `start` for the reason `elapsed` above it is — the decision is
/// worth testing and the thing that produces the reports needs an audio device.
///
/// Three ways out, and only one of them is the full count:
///
/// A closed channel is a thread that died without reporting. Every worker sends
/// once and drops its sender straight after, so the channel running dry early
/// means nothing more is coming — and what was never reported was never opened.
///
/// A timeout is a backend still inside its own `open`, and it is the one thing
/// this cannot get an answer about. Waiting longer is a Record button that
/// never comes back — this runs on the thread that answers it. So the recording
/// runs, which is what the recorder did before the handshake existed.
///
/// Not refused, though the failing case above is, and the difference is what
/// happens to the thread afterwards. A worker blocked inside a foreign
/// blocking call cannot be cancelled: `stop_flag` is only read once it returns.
/// Letting the recording start leaves that thread owned — its handle is stored,
/// and `stop` joins it, so whenever the driver comes back it winds down into
/// the meeting it belongs to. Refusing would drop the handle with the thread
/// still inside the backend, and it would surface later, appending to whatever
/// recording happened to be running by then. A stalled device is rare; a
/// recording carrying a few seconds of a previous one is not something the user
/// could ever untangle.
fn streams_are_running(ready: &mpsc::Receiver<bool>, expected: usize, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut opened = 0usize;
    for _ in 0..expected {
        match ready.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(true) => opened += 1,
            Ok(false) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => return true,
        }
    }
    opened > 0
}

/// Copy samples from `read_pos` to end, then advance the cursor.
/// Full `samples` buffer is left intact for final WAV persistence.
pub fn drain_stt_window(samples: &[i16], read_pos: &mut usize) -> Vec<i16> {
    if *read_pos > samples.len() {
        *read_pos = samples.len();
    }
    let out = samples[*read_pos..].to_vec();
    *read_pos = samples.len();
    out
}

impl Default for DualChannelRecorder {
    fn default() -> Self {
        Self::new()
    }
}

fn f32_to_i16_mono(data: &[f32], frames: usize, channels: usize) -> Vec<i16> {
    let ch = channels.max(1);
    let mut out = Vec::with_capacity(frames);
    if ch == 1 {
        for &s in data.iter().take(frames) {
            out.push((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
        }
    } else {
        for frame in data.chunks(ch).take(frames) {
            let sum: f32 = frame.iter().sum();
            let avg = sum / ch as f32;
            out.push((avg.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
        }
    }
    out
}

/// Always two channels, including when only one of them was ever captured.
///
/// A recording with a channel switched off writes silence into that channel
/// rather than coming out as a mono file. Everything downstream is built on
/// L=Me, R=Others — `read_dual_wav`, the player, the waveform, the final pass —
/// and a mono file walks into `read_dual_wav`'s single-channel branch, which
/// hands the samples back as Me with Others silent. On a system-only recording
/// that is not merely a smaller file: it is the other side of the meeting
/// attributed to the user, in the transcript and in every export made from it.
///
/// Silence costs two bytes a frame and nothing else. It is not transcribed
/// either — `transcribe_channel` and `whole_channel` both floor a channel with
/// no peak in it before a decoder is opened, so an hour of zeros is skipped
/// rather than listened to.
pub fn write_dual_wav(
    path: &Path,
    sample_rate: u32,
    mic: &[i16],
    system: &[i16],
) -> Result<(), CaptureError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        WavWriter::create(path, spec).map_err(|e| CaptureError::Device(e.to_string()))?;
    let merged = merge_dual_channel(mic, system);
    for chunk in merged.chunks(2) {
        let l = chunk.first().map(|(_, s)| *s).unwrap_or(0);
        let r = chunk.get(1).map(|(_, s)| *s).unwrap_or(0);
        writer
            .write_sample(l)
            .map_err(|e| CaptureError::Device(e.to_string()))?;
        writer
            .write_sample(r)
            .map_err(|e| CaptureError::Device(e.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|e| CaptureError::Device(e.to_string()))?;
    Ok(())
}

/// Read a wav file into mono i16 samples (downmix).
pub fn read_wav_mono(path: &Path) -> Result<(Vec<i16>, u32), CaptureError> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| CaptureError::Device(e.to_string()))?;
    reject_overlong_wav(&reader)?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let samples: Result<Vec<i16>, _> = match spec.sample_format {
        hound::SampleFormat::Int => reader.samples::<i16>().collect(),
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|r| r.map(|f| (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16))
            .collect(),
    };
    let samples = samples.map_err(|e| CaptureError::Device(e.to_string()))?;
    let mut mono = Vec::with_capacity(samples.len() / channels + 1);
    for frame in samples.chunks(channels) {
        let sum: i32 = frame.iter().map(|s| *s as i32).sum();
        mono.push((sum / channels as i32) as i16);
    }
    Ok((mono, spec.sample_rate))
}

/// Rejects a WAV whose declared length exceeds the decoder's ceiling.
///
/// Reading the header costs nothing and covers every encoding the reader accepts.
/// Guessing from the file size does not: 8-bit mono is one byte per sample on
/// disk and two in memory, so a byte-based bound lets four times the intended
/// number of samples through.
fn reject_overlong_wav(
    reader: &hound::WavReader<std::io::BufReader<std::fs::File>>,
) -> Result<(), CaptureError> {
    let channels = reader.spec().channels.max(1) as usize;
    let frames = reader.len() as usize / channels;
    if frames > crate::audio::decode::MAX_DECODED_SAMPLES {
        return Err(CaptureError::Device(format!(
            "audio is longer than {} hours; split it into shorter recordings",
            crate::audio::decode::MAX_DECODED_SAMPLES / (60 * 60 * 48_000)
        )));
    }
    Ok(())
}

/// Split stereo dual-channel WAV (L=Me, R=Others) into separate mono buffers.
pub fn read_dual_wav(path: &Path) -> Result<(Vec<i16>, Vec<i16>, u32), CaptureError> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| CaptureError::Device(e.to_string()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let samples: Result<Vec<i16>, _> = match spec.sample_format {
        hound::SampleFormat::Int => reader.samples::<i16>().collect(),
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|r| r.map(|f| (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16))
            .collect(),
    };
    let samples = samples.map_err(|e| CaptureError::Device(e.to_string()))?;
    if channels >= 2 {
        let mut mic = Vec::new();
        let mut sys = Vec::new();
        for frame in samples.chunks(channels) {
            mic.push(frame[0]);
            sys.push(*frame.get(1).unwrap_or(&0));
        }
        Ok((mic, sys, spec.sample_rate))
    } else {
        Ok((samples.clone(), vec![0; samples.len()], spec.sample_rate))
    }
}

#[cfg(test)]
mod tests {

    /// A failed transcription must not cost the meeting its audio.
    #[test]
    fn rewinding_hands_the_same_chunk_back_to_the_next_poll() {
        let rec = DualChannelRecorder::new();
        rec.push_samples_for_test(&[7i16; 1600], &[9i16; 1600]);

        let (mic, sys, _) = rec.drain_chunks();
        assert_eq!(mic.len(), 1600);
        // Draining again with nothing new gives nothing — the cursor moved.
        assert!(rec.drain_chunks().0.is_empty());

        rec.rewind_chunks(mic.len(), sys.len());
        let (again, _, _) = rec.drain_chunks();
        assert_eq!(
            again.len(),
            1600,
            "the audio a failed transcription was holding must come back"
        );
        assert_eq!(again[0], 7);
    }

    /// Rewinding more than was ever taken lands at the start rather than
    /// underflowing, which on a `usize` would be a panic in release and a
    /// catastrophic cursor in debug.
    #[test]
    fn rewinding_past_the_beginning_is_not_an_underflow() {
        let rec = DualChannelRecorder::new();
        rec.push_samples_for_test(&[1i16; 100], &[1i16; 100]);
        let _ = rec.drain_chunks();
        rec.rewind_chunks(usize::MAX, usize::MAX);
        assert_eq!(rec.drain_chunks().0.len(), 100);
    }
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn entering_pause_drains_then_stops_the_stream() {
        // Only the false→true edge may touch the backend; every later paused
        // tick has to leave the already-stopped stream alone.
        assert_eq!(capture_step(true, false), StreamAction::EnterPause);
        assert_eq!(capture_step(true, true), StreamAction::StayPaused);
    }

    #[test]
    fn leaving_pause_resumes_before_polling() {
        assert_eq!(capture_step(false, true), StreamAction::LeavePause);
        assert_eq!(capture_step(false, false), StreamAction::Poll);
    }

    #[test]
    fn a_paused_clock_does_not_advance() {
        let start = Instant::now();
        let now = start + Duration::from_secs(30);
        // 10s recorded, then 30s of wall clock spent paused.
        assert_eq!(elapsed(Some(start), 10_000, true, now), 10_000);
        assert_eq!(elapsed(Some(start), 10_000, false, now), 40_000);
        // `pause()` takes `start`, so a paused recorder has none to run from.
        assert_eq!(elapsed(None, 10_000, false, now), 10_000);
    }

    #[test]
    fn write_and_read_wav_roundtrip_preserves_channels() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let mic = vec![0i16, 1000, -1000, 500];
        let sys = vec![0i16, 200, -200, 100];
        write_dual_wav(&path, 16_000, &mic, &sys).unwrap();
        let (m2, s2, sr) = read_dual_wav(&path).unwrap();
        assert_eq!(sr, 16_000);
        assert_eq!(m2, mic);
        assert_eq!(s2, sys);
    }

    /// One stream that opened is a recording; none is not. The mixed case is
    /// what keeps a two-channel meeting whose microphone failed working exactly
    /// as it does today — half a recording rather than a refusal.
    #[test]
    fn a_recording_needs_one_stream_that_opened() {
        let (tx, rx) = mpsc::channel();
        tx.send(false).unwrap();
        tx.send(true).unwrap();
        assert!(streams_are_running(&rx, 2, STREAM_READY_TIMEOUT));

        let (tx, rx) = mpsc::channel();
        tx.send(false).unwrap();
        drop(tx);
        assert!(!streams_are_running(&rx, 1, STREAM_READY_TIMEOUT));
    }

    /// A worker that died before reporting must not leave the start waiting on
    /// a message nobody is left to send.
    #[test]
    fn a_worker_that_never_reported_does_not_hang_the_start() {
        let (tx, rx) = mpsc::channel();
        tx.send(true).unwrap();
        // One reported and let go of its sender; the other went down without
        // saying anything, which is the channel closing a report short.
        drop(tx);
        assert!(streams_are_running(&rx, 2, STREAM_READY_TIMEOUT));
    }

    /// A backend wedged inside its own `open` never reports and never lets go
    /// of its sender, so the wait has to end on its own — this runs on the
    /// thread that answers the Record button. It ends the way the recorder
    /// behaved before the handshake existed: the recording runs.
    #[test]
    fn a_backend_that_never_answers_lets_the_recording_run() {
        // Held, exactly as a stalled worker holds it: the channel neither
        // delivers nor disconnects.
        let (_tx, rx) = mpsc::channel::<bool>();
        assert!(streams_are_running(&rx, 1, Duration::from_millis(10)));
    }

    /// Neither channel selected opens no device at all, and the recorder says
    /// so rather than running with no worker behind it. `domain::gate` refuses
    /// this before the user can reach it; this is the floor under that.
    #[test]
    fn starting_with_both_channels_off_is_refused() {
        let rec = DualChannelRecorder::new();
        let dir = tempdir().unwrap();
        let started = rec.start(
            dir.path().join("nothing.wav"),
            ChannelSelection {
                me: false,
                others: false,
            },
            None,
            None,
        );
        assert!(started.is_err());
        assert!(!rec.is_recording());
    }

    /// A channel that was switched off never produced a sample, and the file
    /// still has to be the stereo one every reader expects — with that channel
    /// silent, and as long as the one that did record.
    #[test]
    fn a_recording_with_one_channel_off_is_still_a_stereo_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mic-only.wav");
        let mic = vec![1000i16; 800];
        write_dual_wav(&path, 16_000, &mic, &[]).unwrap();
        let (m, s, _) = read_dual_wav(&path).unwrap();
        assert_eq!(m, mic);
        assert_eq!(
            s,
            vec![0i16; 800],
            "the absent channel must be silence of the same length, not missing"
        );

        // And the other way round, which is the case a mono file would get
        // wrong: these samples belong to Others and must not come back as Me.
        let path = dir.path().join("system-only.wav");
        let sys = vec![-1000i16; 800];
        write_dual_wav(&path, 16_000, &[], &sys).unwrap();
        let (m, s, _) = read_dual_wav(&path).unwrap();
        assert_eq!(m, vec![0i16; 800]);
        assert_eq!(s, sys);
    }

    #[test]
    fn system_loopback_source_kind_is_not_mic() {
        // Compile-time / API guarantee: we open SystemLoopback for Others.
        assert_ne!(
            format!("{:?}", SourceKind::SystemLoopback),
            format!("{:?}", SourceKind::Mic)
        );
    }

    #[test]
    fn drain_stt_window_preserves_full_recording() {
        let mut samples = vec![1i16, 2, 3, 4, 5, 6];
        let mut pos = 0usize;
        let w1 = drain_stt_window(&samples, &mut pos);
        assert_eq!(w1, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(pos, 6);
        // Full buffer still intact
        assert_eq!(samples, vec![1, 2, 3, 4, 5, 6]);
        // Second drain empty until new samples
        assert!(drain_stt_window(&samples, &mut pos).is_empty());
        samples.extend_from_slice(&[7, 8]);
        let w2 = drain_stt_window(&samples, &mut pos);
        assert_eq!(w2, vec![7, 8]);
        assert_eq!(samples.len(), 8);
    }

    #[test]
    fn live_stt_drain_does_not_truncate_final_wav() {
        // Simulates poll_live_stt calling drain_chunks repeatedly while recording.
        let rec = DualChannelRecorder::new();
        rec.push_samples_for_test(&[100i16; 1600], &[200i16; 1600]); // ~100ms @16k
        let (d1_m, d1_s, _) = rec.drain_chunks();
        assert_eq!(d1_m.len(), 1600);
        assert_eq!(d1_s.len(), 1600);

        rec.push_samples_for_test(&[101i16; 1600], &[201i16; 1600]);
        let (d2_m, _, _) = rec.drain_chunks();
        assert_eq!(d2_m.len(), 1600);

        rec.push_samples_for_test(&[102i16; 800], &[202i16; 800]);
        // After three live-STT drains, full recording must still be 1600*2+800
        let (mic_len, sys_len) = rec.recording_len();
        assert_eq!(mic_len, 4000);
        assert_eq!(sys_len, 4000);

        let dir = tempdir().unwrap();
        let path = dir.path().join("meeting.wav");
        rec.flush_recording_for_test(&path).unwrap();
        let (mic, sys, _) = read_dual_wav(&path).unwrap();
        assert_eq!(
            mic.len(),
            4000,
            "final WAV must keep all samples after live STT drains"
        );
        assert_eq!(sys.len(), 4000);
        // Spot-check channels still distinct (Me vs Others)
        assert_eq!(mic[0], 100);
        assert_eq!(sys[0], 200);
        assert_eq!(mic[3200], 102);
        assert_eq!(sys[3200], 202);
    }
}
