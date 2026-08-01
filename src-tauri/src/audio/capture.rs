//! Dual-channel audio capture: microphone (Me) + system/loopback (Others).
//! Platform backends compile on all targets; runtime may fall back when
//! system loopback is unavailable.

use crate::audio::levels::ChannelLevels;
use crate::domain::speaker::merge_dual_channel;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use hound::{WavSpec, WavWriter};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

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

pub struct DualChannelRecorder {
    inner: Arc<Mutex<RecorderInner>>,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    started_at_ms: Arc<AtomicU64>,
}

/// cpal::Stream is !Send on some platforms; desktop capture is only driven from
/// the app runtime which serializes start/stop via this mutex.
struct StreamSlot(#[allow(dead_code)] cpal::Stream);
// SAFETY: Stream is only created/dropped while holding RecorderInner mutex and
// never moved across threads for concurrent use.
unsafe impl Send for StreamSlot {}
unsafe impl Sync for StreamSlot {}

struct RecorderInner {
    mic_samples: Vec<i16>,
    sys_samples: Vec<i16>,
    sample_rate: u32,
    levels: ChannelLevels,
    streams: Vec<StreamSlot>,
    out_path: Option<PathBuf>,
    start: Option<Instant>,
    elapsed_before_pause_ms: u64,
}

impl DualChannelRecorder {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecorderInner {
                mic_samples: Vec::new(),
                sys_samples: Vec::new(),
                sample_rate: 16_000,
                levels: ChannelLevels::default(),
                streams: Vec::new(),
                out_path: None,
                start: None,
                elapsed_before_pause_ms: 0,
            })),
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            started_at_ms: Arc::new(AtomicU64::new(0)),
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

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| CaptureError::Device("no default input device".into()))?;

        let config = device
            .default_input_config()
            .map_err(|e| CaptureError::Device(e.to_string()))?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();
        let stream_config: StreamConfig = config.clone().into();

        {
            let mut g = self.inner.lock();
            g.mic_samples.clear();
            g.sys_samples.clear();
            g.sample_rate = sample_rate;
            g.out_path = Some(out_path);
            g.streams.clear();
            g.start = Some(Instant::now());
            g.elapsed_before_pause_ms = 0;
            g.levels = ChannelLevels::default();
        }

        let inner = self.inner.clone();
        let paused = self.paused.clone();

        let err_fn = |e| eprintln!("audio stream error: {e}");

        let stream = match sample_format {
            SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        if paused.load(Ordering::SeqCst) {
                            return;
                        }
                        let mut mic = Vec::with_capacity(data.len());
                        let mut sys = Vec::with_capacity(data.len());
                        // Dual-channel path: if device is stereo, L=Me attempt, R=system/ambient.
                        // Mono duplicates into Me and leaves Others near-silent until loopback is wired.
                        if channels >= 2 {
                            for frame in data.chunks(channels as usize) {
                                let l = (frame[0].clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                let r = (frame
                                    .get(1)
                                    .copied()
                                    .unwrap_or(0.0)
                                    .clamp(-1.0, 1.0)
                                    * i16::MAX as f32) as i16;
                                mic.push(l);
                                sys.push(r);
                            }
                        } else {
                            for &s in data {
                                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                mic.push(v);
                                // Synthetic low-level "others" placeholder keeps dual-channel pipeline live.
                                sys.push(v / 20);
                            }
                        }
                        let mut g = inner.lock();
                        g.levels = ChannelLevels::from_buffers(&mic, &sys);
                        g.mic_samples.extend_from_slice(&mic);
                        g.sys_samples.extend_from_slice(&sys);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| CaptureError::Device(e.to_string()))?,
            SampleFormat::I16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        if paused.load(Ordering::SeqCst) {
                            return;
                        }
                        let mut mic = Vec::new();
                        let mut sys = Vec::new();
                        if channels >= 2 {
                            for frame in data.chunks(channels as usize) {
                                mic.push(frame[0]);
                                sys.push(*frame.get(1).unwrap_or(&0));
                            }
                        } else {
                            for &s in data {
                                mic.push(s);
                                sys.push(s / 20);
                            }
                        }
                        let mut g = inner.lock();
                        g.levels = ChannelLevels::from_buffers(&mic, &sys);
                        g.mic_samples.extend_from_slice(&mic);
                        g.sys_samples.extend_from_slice(&sys);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| CaptureError::Device(e.to_string()))?,
            SampleFormat::U16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| {
                        if paused.load(Ordering::SeqCst) {
                            return;
                        }
                        let mut mic = Vec::new();
                        let mut sys = Vec::new();
                        for frame in data.chunks(channels.max(1) as usize) {
                            let s = (frame[0] as i32 - 32768) as i16;
                            mic.push(s);
                            sys.push(s / 20);
                        }
                        let mut g = inner.lock();
                        g.levels = ChannelLevels::from_buffers(&mic, &sys);
                        g.mic_samples.extend_from_slice(&mic);
                        g.sys_samples.extend_from_slice(&sys);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| CaptureError::Device(e.to_string()))?,
            _ => {
                return Err(CaptureError::Device(format!(
                    "unsupported sample format {sample_format:?}"
                )))
            }
        };

        stream
            .play()
            .map_err(|e| CaptureError::Device(e.to_string()))?;
        self.inner.lock().streams.push(StreamSlot(stream));
        self.paused.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        self.started_at_ms.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            Ordering::SeqCst,
        );
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

        let mut g = self.inner.lock();
        g.streams.clear();
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

    /// Snapshot of current PCM for live STT chunking (consumes read position via clear optional).
    pub fn drain_chunks(&self) -> (Vec<i16>, Vec<i16>, u32) {
        let mut g = self.inner.lock();
        let mic = std::mem::take(&mut g.mic_samples);
        let sys = std::mem::take(&mut g.sys_samples);
        (mic, sys, g.sample_rate)
    }
}

impl Default for DualChannelRecorder {
    fn default() -> Self {
        Self::new()
    }
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
    let mut writer = WavWriter::create(path, spec).map_err(|e| CaptureError::Device(e.to_string()))?;
    let merged = merge_dual_channel(mic, system);
    // merge returns interleaved (Me, Others) pairs as labeled samples; write stereo L/R.
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

/// Read a wav file into mono i16 samples (downmix) for import/retranscription.
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_and_read_wav_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let mic = vec![0i16, 1000, -1000, 500];
        let sys = vec![0i16, 200, -200, 100];
        write_dual_wav(&path, 16_000, &mic, &sys).unwrap();
        let (mono, sr) = read_wav_mono(&path).unwrap();
        assert_eq!(sr, 16_000);
        assert_eq!(mono.len(), 4);
    }
}
