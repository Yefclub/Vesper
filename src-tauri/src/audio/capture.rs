//! Dual-channel audio capture: microphone (Me) + system loopback (Others).
//! Uses flexaudio backends: WASAPI loopback (Windows), CoreAudio taps (macOS),
//! PipeWire (Linux) for system audio — never fakes Others from the mic.

use crate::audio::levels::ChannelLevels;
use crate::domain::channels::ChannelSelection;
use crate::domain::recovery::{wav_header_fix, RIFF_SIZE_OFFSET};
use crate::domain::segmenter::has_speech;
use crate::domain::speaker::merge_dual_channel;
use flexaudio::{open, OutputFormat, SourceKind, StreamConfig};
use hound::{WavSpec, WavWriter};
use parking_lot::Mutex;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
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
    /// Whether each channel has ever delivered a chunk above the floor since
    /// this recording started. Latched, never cleared mid-recording: the
    /// question is "has this input ever produced sound", and one yes settles it.
    heard_me: bool,
    heard_others: bool,
    /// When the first of them first heard something, on this recording's clock.
    /// The short wait for a dead channel is measured from here rather than from
    /// the start — see `domain::deaf`.
    evidence_at_ms: Option<u64>,
    /// The channels this recording was started with.
    ///
    /// Kept here rather than read back from settings, because the two stop
    /// agreeing the moment a recording is running: `start` copies the selection
    /// into the streams, so a switch flipped afterwards belongs to the next
    /// recording. A meter reading zero has two meanings — nobody is talking,
    /// or nothing is listening — and this is what tells them apart.
    channels: ChannelSelection,
    /// Whether each capture thread has finished. A channel that has ended will
    /// never produce another sample, which is what lets the other one carry on
    /// being written — see `flush_frontier`.
    ///
    /// A channel that is switched off starts out already ended: it has no
    /// thread to finish and will never produce a sample either, which is the
    /// same thing as far as the file is concerned.
    mic_ended: bool,
    sys_ended: bool,
}

/// The recording's file, and how far into the capture it has been written.
///
/// Behind a lock of its own rather than inside `RecorderInner`, and that is the
/// whole point of it being a separate type: writing is the one thing here that
/// can block on hardware, and both capture threads need `inner` before they can
/// hand over the chunks they have just polled. Holding one lock across a disk
/// write would stall them, and flexaudio's ring drops its oldest chunk when
/// nobody is draining it — so a slow disk would cost the meeting exactly the
/// audio this exists to save.
///
/// Anything that touches both takes this one first.
struct FileSink {
    /// `None` before a recording, once `stop` has closed it, and after a write
    /// that failed — which is what selects the whole-buffer fallback, so the
    /// meeting survives a disk that refused this.
    writer: Option<WavWriter<BufWriter<File>>>,
    /// How many interleaved frames of the recording are already in the file.
    written: usize,
}

pub struct DualChannelRecorder {
    inner: Arc<Mutex<RecorderInner>>,
    sink: Mutex<FileSink>,
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
                heard_me: false,
                heard_others: false,
                evidence_at_ms: None,
                channels: ChannelSelection::default(),
                mic_ended: false,
                sys_ended: false,
            })),
            sink: Mutex::new(FileSink {
                writer: None,
                written: 0,
            }),
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

    /// Whether each channel has ever delivered sound in this recording, and when
    /// the first of them did.
    pub fn heard(&self) -> (bool, bool, Option<u64>) {
        let g = self.inner.lock();
        (g.heard_me, g.heard_others, g.evidence_at_ms)
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
        // The file is opened here, before a single sample exists, and written
        // as the meeting goes on. It used to be created in `stop`, which meant
        // a two-hour meeting had nothing at all on disk until the moment the
        // user pressed the button — so a crash cost all of it rather than the
        // last write. Failing here rather than there is also the better place
        // to find out: a recording that cannot be saved should refuse to start,
        // not discover it two hours later.
        let writer = open_dual_wav(&out_path, sample_rate)?;
        {
            // The file's lock before the capture one, the order everything
            // here takes them in.
            let mut sink = self.sink.lock();
            sink.writer = Some(writer);
            sink.written = 0;
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
            g.heard_me = false;
            g.heard_others = false;
            g.evidence_at_ms = None;
            g.channels = channels;
            // A channel that is off is ended before it began — no thread to
            // finish, and never a sample to come. `flush_frontier` holds the
            // file back to whatever both channels have reached, so left false
            // this would sit at zero for the whole meeting: a mic-only
            // recording would reach the disk only at Stop, which is exactly the
            // crash the streaming write exists to survive.
            g.mic_ended = !channels.me;
            g.sys_ended = !channels.others;
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
                            tracing::error!("mic open failed: {e}");
                            // Said out loud on every way out, because the streaming
                            // write holds the file back to whatever both channels
                            // have reached. A microphone that never opened would
                            // otherwise hold it at nothing for the whole meeting,
                            // and the system audio would never reach disk either.
                            inner_mic.lock().mic_ended = true;
                            let _ = ready.send(false);
                            return;
                        }
                    };
                    if let Err(e) = stream.start() {
                        tracing::error!("mic start failed: {e}");
                        inner_mic.lock().mic_ended = true;
                        let _ = ready.send(false);
                        return;
                    }
                    let _ = ready.send(true);
                    // Dropped here rather than at the end of the thread, and
                    // load-bearing: a worker that dies without reporting has to
                    // show up as the channel closing. Held for the length of the
                    // capture, the surviving worker's copy would keep it open and
                    // `start` would wait for a message nobody is left to send.
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
                            // Latched here rather than sampled off the meter,
                            // and that is the difference between working and not:
                            // the meter holds one chunk and is overwritten every
                            // 20ms, so a reader on the 1200ms tick sees one chunk
                            // in sixty and would call a channel dead over the
                            // fifty-nine it never looked at.
                            if crate::domain::deaf::heard(chunk.peak) && g.evidence_at_ms.is_none() {
                                let at = elapsed(
                                    g.start,
                                    g.elapsed_before_pause_ms,
                                    paused.load(Ordering::SeqCst),
                                    Instant::now(),
                                );
                                g.evidence_at_ms = Some(at);
                            }
                            g.heard_me |= crate::domain::deaf::heard(chunk.peak);
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
                    inner_mic.lock().mic_ended = true;
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
                            // Through `tracing`, not `eprintln`. A windowed
                            // Windows build has no stderr, so this failure was
                            // written to nowhere: the recording came back silent
                            // and the log file the application otherwise keeps
                            // had not one line about it.
                            tracing::error!("system loopback open failed: {e}");
                            // See the mic thread: a machine with no loopback at all
                            // is the ordinary case here, and without this the
                            // microphone's audio would never be written either.
                            inner_sys.lock().sys_ended = true;
                            let _ = ready.send(false);
                            return;
                        }
                    };
                    if let Err(e) = stream.start() {
                        tracing::error!("system loopback start failed: {e}");
                        inner_sys.lock().sys_ended = true;
                        let _ = ready.send(false);
                        return;
                    }
                    // See the microphone worker above for why the sender goes as
                    // soon as it has spoken.
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
                            if crate::domain::deaf::heard(chunk.peak) && g.evidence_at_ms.is_none() {
                                let at = elapsed(
                                    g.start,
                                    g.elapsed_before_pause_ms,
                                    paused_sys.load(Ordering::SeqCst),
                                    Instant::now(),
                                );
                                g.evidence_at_ms = Some(at);
                            }
                            g.heard_others |= crate::domain::deaf::heard(chunk.peak);
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
                    let mut g = inner_sys.lock();
                    g.system_loopback_active = false;
                    g.sys_ended = true;
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
            // has already returned, which is what it just reported. The file is
            // another matter — it was created before any of this, so it is
            // closed and taken away rather than left as a headerful of nothing
            // under a meeting that never started.
            self.discard_file();
            return Err(CaptureError::Device(
                "no capture device could be opened".into(),
            ));
        }

        *self.workers.lock() = workers;

        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Drop the file a refused start had already created.
    ///
    /// `open_dual_wav` runs before the streams are known to work, so a start
    /// that gives up leaves a valid, empty WAV behind at the new meeting's
    /// path. No row points at it — the meeting is written only once `start`
    /// succeeds — so nothing would ever offer it, play it or clean it up.
    fn discard_file(&self) {
        let mut sink = self.sink.lock();
        sink.writer = None;
        sink.written = 0;
        // Taken from the recorder rather than passed in, so this cannot be
        // pointed at a file that is not the one just opened.
        if let Some(path) = self.inner.lock().out_path.take() {
            let _ = std::fs::remove_file(path);
        }
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

        {
            let mut g = self.inner.lock();
            if let Some(start) = g.start.take() {
                g.elapsed_before_pause_ms += start.elapsed().as_millis() as u64;
            }
        }
        self.close_file()
    }

    /// Write everything both channels have reached, and put the file's header
    /// in step with it.
    ///
    /// Called from the flush ticker, never from a capture thread: that thread
    /// is the one in this application that must not stall, and a disk that
    /// takes 200ms to answer would cost it a chunk of the meeting.
    pub fn flush_to_disk(&self) {
        let mut sink = self.sink.lock();
        if let Err(e) = self.append(&mut sink, Frontier::WhileCapturing) {
            // Not fatal, and deliberately so: the buffers are still whole in
            // memory and `close_file` writes the recording in one go when the
            // streaming writer is gone, which is exactly what this did before
            // anything was streamed. Dropping the writer is what selects that
            // fallback, and it also stops a file already refused from being
            // appended to for the rest of the meeting.
            tracing::warn!("the recording could not be written as it was captured: {e}");
            sink.writer = None;
        }
    }

    /// Copy out the frames the file does not have yet, then write them with the
    /// capture lock released.
    ///
    /// The copy is the point. Everything a capture thread produces has to pass
    /// through `inner`, so anything held across a write is time those threads
    /// are not draining the backend's ring — and what falls out of a ring
    /// nobody drains is the recording.
    fn append(&self, sink: &mut FileSink, frontier: Frontier) -> Result<(), CaptureError> {
        if sink.writer.is_none() {
            return Ok(());
        }
        let (frames, reached) = {
            let g = self.inner.lock();
            let reached = match frontier {
                Frontier::WhileCapturing => flush_frontier(
                    g.mic_samples.len(),
                    g.sys_samples.len(),
                    g.mic_ended,
                    g.sys_ended,
                ),
                // Both capture threads have been joined by the time this is
                // asked for, so every sample either of them will ever produce
                // is already here and the shorter channel is padded with
                // silence exactly as `write_dual_wav` would pad it.
                Frontier::Everything => g.mic_samples.len().max(g.sys_samples.len()),
            };
            if reached <= sink.written {
                return Ok(());
            }
            // Both slices start at the same frame, so `merge_dual_channel` pads
            // the short one exactly where `write_dual_wav` would have — which
            // is what makes the streamed file byte-for-byte the one the single
            // write produced.
            let from = sink.written;
            let mic =
                &g.mic_samples[from.min(g.mic_samples.len())..reached.min(g.mic_samples.len())];
            let sys =
                &g.sys_samples[from.min(g.sys_samples.len())..reached.min(g.sys_samples.len())];
            (merge_dual_channel(mic, sys), reached)
        };
        let Some(writer) = sink.writer.as_mut() else {
            return Ok(());
        };
        for (_, sample) in frames {
            writer
                .write_sample(sample)
                .map_err(|e| CaptureError::Device(e.to_string()))?;
        }
        sink.written = reached;
        // `hound`'s flush rewrites the length in the header as well as emptying
        // the buffer, so what is on disk after this is a whole, readable WAV
        // rather than a header from the start of the meeting with bytes behind
        // it.
        writer
            .flush()
            .map_err(|e| CaptureError::Device(e.to_string()))?;
        Ok(())
    }

    /// Finish the recording's file and answer with where it is.
    fn close_file(&self) -> Result<PathBuf, CaptureError> {
        let mut sink = self.sink.lock();
        let path = self
            .inner
            .lock()
            .out_path
            .clone()
            .ok_or_else(|| CaptureError::Device("missing output path".into()))?;
        if let Err(e) = self.append(&mut sink, Frontier::Everything) {
            tracing::warn!("the recording's last frames could not be appended: {e}");
            sink.writer = None;
        }
        match sink.writer.take() {
            Some(writer) => writer
                .finalize()
                .map_err(|e| CaptureError::Device(e.to_string()))?,
            // Never opened, or a write along the way failed. The whole meeting
            // is in memory either way, so the file is written in one go — the
            // path this took before any of it was streamed.
            None => {
                let g = self.inner.lock();
                write_dual_wav(&path, g.sample_rate, &g.mic_samples, &g.sys_samples)?
            }
        }
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

    /// Test/helper: open the streaming file as `start` does, without a device.
    ///
    /// The selection comes in because `start` seeds `mic_ended`/`sys_ended`
    /// from it, and that seeding is the whole of what makes a channel that is
    /// off reach the disk. A helper that left the flags alone would test a
    /// frontier no recording ever has.
    #[cfg(test)]
    fn begin_file_for_test(&self, path: &Path, sample_rate: u32, channels: ChannelSelection) {
        let mut sink = self.sink.lock();
        sink.writer = Some(open_dual_wav(path, sample_rate).unwrap());
        sink.written = 0;
        let mut g = self.inner.lock();
        g.sample_rate = sample_rate;
        g.out_path = Some(path.to_path_buf());
        g.channels = channels;
        g.mic_ended = !channels.me;
        g.sys_ended = !channels.others;
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

/// How far into the two channels the file may be written while capture runs.
///
/// A frame carries one sample from each channel, so while both streams are live
/// the boundary is whichever has delivered less: writing past it would put the
/// far channel's later samples at an earlier moment than they were spoken, and
/// they can only be appended once.
///
/// A stream that has *ended* will never deliver again, so the silence in its
/// place from there on is final and stops holding the other back. That case is
/// not exotic — a machine with no system loopback is the ordinary one on Linux
/// — and without it the boundary would sit at zero for the whole meeting and
/// nothing would be streamed at all.
fn flush_frontier(mic_len: usize, sys_len: usize, mic_ended: bool, sys_ended: bool) -> usize {
    match (mic_ended, sys_ended) {
        (false, false) => mic_len.min(sys_len),
        (false, true) => mic_len,
        (true, false) => sys_len,
        (true, true) => mic_len.max(sys_len),
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

/// How much of the capture a write is allowed to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frontier {
    /// A tick during the meeting: only what both channels have delivered.
    WhileCapturing,
    /// The close, once the capture threads are joined and nothing more is
    /// coming: everything, short channel padded.
    Everything,
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

/// The one shape a Vesper recording has: left is Me, right is Others.
///
/// Named because two writers have to agree on it — the one that streams the
/// meeting as it happens and the one that writes it in a single pass when the
/// first could not be opened.
fn dual_wav_spec(sample_rate: u32) -> WavSpec {
    WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

/// Create the recording's file and write its header, ready to be appended to.
fn open_dual_wav(
    path: &Path,
    sample_rate: u32,
) -> Result<WavWriter<BufWriter<File>>, CaptureError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    WavWriter::create(path, dual_wav_spec(sample_rate))
        .map_err(|e| CaptureError::Device(e.to_string()))
}

/// How much of a file to read looking for its two length fields. A WAV written
/// here has both inside the first 44 bytes; the slack is for one somebody
/// else's tool wrote with a chunk of its own in front of the audio.
const WAV_HEAD_BYTES: u64 = 4_096;

/// Make a recording whose writer never finished readable again.
///
/// A process that is killed mid-meeting leaves a header from the last flush
/// with real audio behind it, and every reader stops where the header says.
/// This is the one thing that recovers those seconds, and it is why an
/// interrupted recording is worth offering back at all.
///
/// Answers whether anything was changed, and treats a file it cannot make sense
/// of as needing nothing: a recording that is already whole is the common case
/// and must not be rewritten on the strength of a misread header.
pub fn repair_wav_header(path: &Path) -> Result<bool, CaptureError> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let len = file.metadata()?.len();
    let mut head = Vec::new();
    Read::by_ref(&mut file)
        .take(WAV_HEAD_BYTES)
        .read_to_end(&mut head)?;
    let Some(fix) = wav_header_fix(&head, len) else {
        return Ok(false);
    };
    file.seek(SeekFrom::Start(RIFF_SIZE_OFFSET))?;
    file.write_all(&fix.riff_size.to_le_bytes())?;
    file.seek(SeekFrom::Start(fix.data_size_offset))?;
    file.write_all(&fix.data_size.to_le_bytes())?;
    file.flush()?;
    Ok(true)
}

/// Always two channels, including when only one of them was ever captured.
///
/// A recording with a channel switched off writes silence into that channel
/// rather than coming out as a mono file. Everything downstream is built on
/// L=Me, R=Others — `read_dual_wav`, the player, the waveform, the final pass,
/// the recovery — and a mono file walks into `read_dual_wav`'s single-channel
/// branch, which hands the samples back as Me with Others silent. On a
/// system-only recording that is not merely a smaller file: it is the other
/// side of the meeting attributed to the user, in the transcript and in every
/// export made from it.
///
/// The streamed writer keeps the same shape without needing to know about it:
/// `merge_dual_channel` pads the absent channel frame by frame exactly as this
/// does, so a file written a flush at a time and one written in a single pass
/// are the same file.
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
    let mut writer = WavWriter::create(path, dual_wav_spec(sample_rate))
        .map_err(|e| CaptureError::Device(e.to_string()))?;
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

    /// The whole promise of the streaming write: what a meeting written in
    /// pieces leaves on disk is the file the single write at the end produced,
    /// byte for byte — including the padding of whichever channel arrived last.
    #[test]
    fn a_recording_written_in_pieces_is_the_same_file_as_one_written_at_the_end() {
        let dir = tempdir().unwrap();
        let mic: Vec<i16> = (0..1_000).map(|i| i as i16).collect();
        // Shorter, and it never catches up — the tail is where the two writers
        // could most easily disagree.
        let sys: Vec<i16> = (0..940).map(|i| -(i as i16)).collect();

        let once = dir.path().join("once.wav");
        write_dual_wav(&once, 16_000, &mic, &sys).unwrap();

        let streamed = dir.path().join("streamed.wav");
        let rec = DualChannelRecorder::new();
        rec.begin_file_for_test(&streamed, 16_000, ChannelSelection::default());
        // Three arrivals, with the two channels never quite in step.
        for (m, s) in [(300usize, 250usize), (700, 700), (1_000, 940)] {
            let (have_m, have_s) = rec.recording_len();
            rec.push_samples_for_test(&mic[have_m..m], &sys[have_s..s]);
            rec.flush_to_disk();
        }
        rec.close_file().unwrap();

        assert_eq!(
            std::fs::read(&once).unwrap(),
            std::fs::read(&streamed).unwrap(),
            "streaming must not change what the meeting's file contains"
        );
    }

    /// A flush that never ran, because a stop landed before the first tick.
    #[test]
    fn closing_without_a_single_flush_still_writes_the_whole_recording() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("short.wav");
        let rec = DualChannelRecorder::new();
        rec.begin_file_for_test(&path, 16_000, ChannelSelection::default());
        rec.push_samples_for_test(&[7i16; 400], &[9i16; 400]);
        rec.close_file().unwrap();
        let (mic, sys, _) = read_dual_wav(&path).unwrap();
        assert_eq!(mic.len(), 400);
        assert_eq!(sys[0], 9);
    }

    /// A channel switched off never delivers, so the frontier can never be
    /// "what both have reached" — `start` marks it ended and the live one
    /// carries the file on its own. Without that the meeting would stream
    /// nothing at all and a crash would cost the whole of it, which is the one
    /// thing the streamed write exists to prevent.
    ///
    /// And what lands is still the stereo file: the absent channel is silence
    /// of the same length, written frame by frame, not a mono file with the
    /// system audio sitting where the user's own voice belongs.
    #[test]
    fn a_channel_that_is_off_does_not_hold_the_streamed_file_at_nothing() {
        let dir = tempdir().unwrap();
        let sys: Vec<i16> = (1..=900).map(|i| i as i16).collect();

        let streamed = dir.path().join("system-only.wav");
        let rec = DualChannelRecorder::new();
        rec.begin_file_for_test(
            &streamed,
            16_000,
            ChannelSelection {
                me: false,
                others: true,
            },
        );
        rec.push_samples_for_test(&[], &sys[..600]);
        rec.flush_to_disk();

        // Mid-meeting, and the point of the test: the file already holds every
        // frame the live channel has delivered.
        let (mic_so_far, sys_so_far, _) = read_dual_wav(&streamed).unwrap();
        assert_eq!(
            sys_so_far.len(),
            600,
            "a live channel must reach the disk while the other is off"
        );
        assert_eq!(mic_so_far, vec![0i16; 600]);

        rec.push_samples_for_test(&[], &sys[600..]);
        rec.close_file().unwrap();
        let (mic, written, _) = read_dual_wav(&streamed).unwrap();
        assert_eq!(written, sys);
        assert_eq!(mic, vec![0i16; 900]);

        // Byte for byte what the single write would have produced, which is the
        // property `write_dual_wav` and the streaming writer have to share.
        let once = dir.path().join("once.wav");
        write_dual_wav(&once, 16_000, &[], &sys).unwrap();
        assert_eq!(
            std::fs::read(&once).unwrap(),
            std::fs::read(&streamed).unwrap()
        );
    }

    /// Both channels live: the file may only reach as far as the one that has
    /// delivered less, because the other's samples can only be appended.
    #[test]
    fn a_live_channel_holds_the_write_back_and_an_ended_one_does_not() {
        assert_eq!(flush_frontier(1_000, 600, false, false), 600);
        // The case that makes this exist: no system loopback on the machine.
        // Holding at zero for the whole meeting would stream nothing at all.
        assert_eq!(flush_frontier(1_000, 0, false, true), 1_000);
        assert_eq!(flush_frontier(0, 1_000, true, false), 1_000);
        // Both joined — everything, with the short side padded.
        assert_eq!(flush_frontier(1_000, 600, true, true), 1_000);
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
    ///
    /// The file goes with the refusal. `start` creates it before it knows
    /// whether anything can be captured, and a meeting that never started has
    /// no row pointing at the leftover — nothing would ever clean it up.
    #[test]
    fn starting_with_both_channels_off_is_refused_and_leaves_no_file() {
        let rec = DualChannelRecorder::new();
        let dir = tempdir().unwrap();
        let path = dir.path().join("nothing.wav");
        let started = rec.start(
            path.clone(),
            ChannelSelection {
                me: false,
                others: false,
            },
            None,
            None,
        );
        assert!(started.is_err());
        assert!(!rec.is_recording());
        assert!(!path.exists(), "a refused start must not leave a file");
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

    /// The file a force-kill leaves: a header from the last flush, with real
    /// audio behind it that nothing is pointing at.
    #[test]
    fn repairing_a_stale_header_recovers_the_audio_written_past_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("killed.wav");
        // The last flush: a whole, valid file of 800 frames.
        write_dual_wav(&path, 16_000, &[100i16; 800], &[200i16; 800]).unwrap();
        // And the 800 frames captured after it, which reached the disk without
        // the header ever being told about them.
        let mut tail = Vec::new();
        for _ in 0..800 {
            tail.extend_from_slice(&101i16.to_le_bytes());
            tail.extend_from_slice(&201i16.to_le_bytes());
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(&tail).unwrap();
        drop(file);

        let (before, _, _) = read_dual_wav(&path).unwrap();
        assert_eq!(before.len(), 800, "the header knows only the last flush");

        assert!(repair_wav_header(&path).unwrap());
        let (mic, sys, _) = read_dual_wav(&path).unwrap();
        assert_eq!(mic.len(), 1_600);
        assert_eq!(sys.len(), 1_600);
        // And the two channels are still the two channels.
        assert_eq!(mic[1_500], 101);
        assert_eq!(sys[1_500], 201);
        // Nothing left to do the second time.
        assert!(!repair_wav_header(&path).unwrap());
    }

    /// A recording that was closed properly must come out of recovery
    /// untouched — repair runs over every meeting that is offered back.
    #[test]
    fn a_finished_recording_is_not_rewritten() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("whole.wav");
        write_dual_wav(&path, 16_000, &[1i16; 400], &[2i16; 400]).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(!repair_wav_header(&path).unwrap());
        assert_eq!(before, std::fs::read(&path).unwrap());
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
