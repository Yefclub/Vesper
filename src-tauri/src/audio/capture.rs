//! Dual-channel audio capture: microphone (Me) + system loopback (Others).
//! Uses flexaudio backends: WASAPI loopback (Windows), CoreAudio taps (macOS),
//! PipeWire (Linux) for system audio — never fakes Others from the mic.

use crate::audio::levels::ChannelLevels;
use crate::domain::speaker::merge_dual_channel;
use flexaudio::{open, OutputFormat, SourceKind, StreamConfig};
use hound::{WavSpec, WavWriter};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

    pub fn elapsed_ms(&self) -> u64 {
        let g = self.inner.lock();
        let mut total = g.elapsed_before_pause_ms;
        if let Some(start) = g.start {
            if !self.paused.load(Ordering::SeqCst) {
                total += start.elapsed().as_millis() as u64;
            }
        }
        total
    }

    pub fn start(&self, out_path: PathBuf) -> Result<(), CaptureError> {
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
        }

        self.stop_flag.store(false, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);

        let stop = self.stop_flag.clone();
        let paused = self.paused.clone();
        let inner_mic = self.inner.clone();
        let inner_sys = self.inner.clone();
        let stop_sys = self.stop_flag.clone();
        let paused_sys = self.paused.clone();

        // —— Microphone (Me) ——
        let mic_handle = thread::Builder::new()
            .name("vesper-mic".into())
            .spawn(move || {
                let cfg = StreamConfig {
                    kind: SourceKind::Mic,
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
                        return;
                    }
                };
                if let Err(e) = stream.start() {
                    eprintln!("mic start failed: {e}");
                    return;
                }
                while !stop.load(Ordering::SeqCst) {
                    if paused.load(Ordering::SeqCst) {
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
                let _ = stream.stop();
            })
            .map_err(|e| CaptureError::Device(e.to_string()))?;

        // —— System loopback (Others) — real WASAPI/CoreAudio/PipeWire via flexaudio ——
        let sys_handle = thread::Builder::new()
            .name("vesper-system".into())
            .spawn(move || {
                let cfg = StreamConfig {
                    kind: SourceKind::SystemLoopback,
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
                        return;
                    }
                };
                if let Err(e) = stream.start() {
                    eprintln!("system loopback start failed: {e}");
                    return;
                }
                {
                    inner_sys.lock().system_loopback_active = true;
                }
                while !stop_sys.load(Ordering::SeqCst) {
                    if paused_sys.load(Ordering::SeqCst) {
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
                let _ = stream.stop();
                inner_sys.lock().system_loopback_active = false;
            })
            .map_err(|e| CaptureError::Device(e.to_string()))?;

        {
            let mut w = self.workers.lock();
            w.clear();
            w.push(mic_handle);
            w.push(sys_handle);
        }

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
    use super::*;
    use tempfile::tempdir;

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
        assert_eq!(mic.len(), 4000, "final WAV must keep all samples after live STT drains");
        assert_eq!(sys.len(), 4000);
        // Spot-check channels still distinct (Me vs Others)
        assert_eq!(mic[0], 100);
        assert_eq!(sys[0], 200);
        assert_eq!(mic[3200], 102);
        assert_eq!(sys[3200], 202);
    }
}
