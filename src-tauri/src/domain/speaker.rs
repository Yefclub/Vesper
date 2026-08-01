use serde::{Deserialize, Serialize};

/// Dual-channel speaker label: microphone = Me, system audio = Others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    Me,
    Others,
}

impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Speaker::Me => "Me",
            Speaker::Others => "Others",
        }
    }

    /// Map capture channel index: 0 = microphone (Me), 1 = system (Others).
    pub fn from_channel(channel: u8) -> Self {
        if channel == 0 {
            Speaker::Me
        } else {
            Speaker::Others
        }
    }
}

/// Merge dual-channel PCM frames into labeled stereo-interleaved frames
/// where left = Me (mic) and right = Others (system). Lengths are equalized
/// by zero-padding the shorter side.
pub fn merge_dual_channel(mic: &[i16], system: &[i16]) -> Vec<(Speaker, i16)> {
    let n = mic.len().max(system.len());
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let m = *mic.get(i).unwrap_or(&0);
        let s = *system.get(i).unwrap_or(&0);
        out.push((Speaker::Me, m));
        out.push((Speaker::Others, s));
    }
    out
}

/// Peak level in 0.0..=1.0 for a PCM buffer (for waveform meters).
pub fn peak_level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    peak as f32 / i16::MAX as f32
}

/// RMS level in 0.0..=1.0 for smoother meters.
pub fn rms_level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| {
        let v = *s as f64 / i16::MAX as f64;
        v * v
    }).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_from_channel() {
        assert_eq!(Speaker::from_channel(0), Speaker::Me);
        assert_eq!(Speaker::from_channel(1), Speaker::Others);
        assert_eq!(Speaker::Me.label(), "Me");
        assert_eq!(Speaker::Others.label(), "Others");
    }

    #[test]
    fn merge_pads_shorter_side() {
        let mic = vec![100i16, 200];
        let sys = vec![50i16];
        let merged = merge_dual_channel(&mic, &sys);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0], (Speaker::Me, 100));
        assert_eq!(merged[1], (Speaker::Others, 50));
        assert_eq!(merged[2], (Speaker::Me, 200));
        assert_eq!(merged[3], (Speaker::Others, 0));
    }

    #[test]
    fn peak_and_rms_bounds() {
        assert_eq!(peak_level(&[]), 0.0);
        assert!((peak_level(&[i16::MAX]) - 1.0).abs() < f32::EPSILON);
        let rms = rms_level(&[0, 0, 0]);
        assert_eq!(rms, 0.0);
    }
}
