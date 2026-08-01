//! Multi-format audio decode → mono i16 PCM for import/retranscription.
//! Supports WAV (hound fast path) and mp3/m4a/ogg/flac/webm via symphonia.

use crate::audio::capture::CaptureError;
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode any supported audio file to mono i16 + sample rate.
/// Ceiling on decoded mono samples, roughly twelve hours at 48 kHz — about 4 GB
/// once the file is expanded into `Vec<i16>`.
///
/// The bound has to be on decoded frames, not on the file: a compressed hour is
/// a few tens of MB on disk and hundreds of MB in memory, so a size check on the
/// input passes files that still exhaust the process while decoding.
pub const MAX_DECODED_SAMPLES: usize = 12 * 60 * 60 * 48_000;

pub fn decode_audio_file(path: &Path) -> Result<(Vec<i16>, u32), CaptureError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Fast path for plain PCM WAV.
    //
    // It reads every interleaved sample before mixing down, so it has to be
    // bounded too. WAV is uncompressed, which makes the file size a faithful
    // stand-in for the sample count: at worst 16-bit stereo, four bytes per mono
    // sample that survives the mixdown.
    if ext == "wav" {
        let bytes = std::fs::metadata(path).map_err(CaptureError::Io)?.len();
        if bytes > (MAX_DECODED_SAMPLES as u64).saturating_mul(4) {
            return Err(CaptureError::Device(format!(
                "audio is longer than {} hours; split it into shorter recordings",
                MAX_DECODED_SAMPLES / (60 * 60 * 48_000)
            )));
        }
        return crate::audio::capture::read_wav_mono(path);
    }

    decode_with_symphonia(path)
}

fn decode_with_symphonia(path: &Path) -> Result<(Vec<i16>, u32), CaptureError> {
    let file = File::open(path).map_err(CaptureError::Io)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| CaptureError::Device(format!("probe audio: {e}")))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| CaptureError::Device("no audio track in file".into()))?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| CaptureError::Device(format!("decoder: {e}")))?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| CaptureError::Device("unknown sample rate".into()))?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut mono: Vec<i16> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::IoError(_)) => break,
            Err(e) => return Err(CaptureError::Device(format!("packet: {e}"))),
        };

        if packet.track_id() != track.id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(CaptureError::Device(format!("decode: {e}"))),
        };

        if sample_buf.is_none() {
            let spec = *decoded.spec();
            let duration = decoded.capacity() as u64;
            sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
        }

        if let Some(buf) = sample_buf.as_mut() {
            buf.copy_interleaved_ref(decoded);
            let samples = buf.samples();
            if channels <= 1 {
                for &s in samples {
                    mono.push((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                }
            } else {
                for frame in samples.chunks(channels) {
                    let sum: f32 = frame.iter().sum();
                    let avg = sum / channels as f32;
                    mono.push((avg.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                }
            }
            // Checked per packet, while the allocation is still bounded. Deciding
            // afterwards would mean the memory was already taken.
            if mono.len() > MAX_DECODED_SAMPLES {
                return Err(CaptureError::Device(format!(
                    "audio is longer than {} hours; split it into shorter recordings",
                    MAX_DECODED_SAMPLES / (60 * 60 * 48_000)
                )));
            }
        }
    }

    if mono.is_empty() {
        return Err(CaptureError::Device(
            "decoded zero samples — unsupported or empty audio".into(),
        ));
    }

    Ok((mono, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::write_dual_wav;
    use tempfile::tempdir;

    #[test]
    fn decodes_wav_via_public_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.wav");
        write_dual_wav(&path, 16_000, &[1000i16; 800], &[500i16; 800]).unwrap();
        let (pcm, sr) = decode_audio_file(&path).unwrap();
        assert_eq!(sr, 16_000);
        assert!(!pcm.is_empty());
    }

    #[test]
    fn rejects_empty_or_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.mp3");
        assert!(decode_audio_file(&path).is_err());
    }
}
