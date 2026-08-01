use crate::domain::speaker::{peak_level, rms_level};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct ChannelLevels {
    pub me_peak: f32,
    pub me_rms: f32,
    pub others_peak: f32,
    pub others_rms: f32,
}

impl ChannelLevels {
    pub fn from_buffers(mic: &[i16], system: &[i16]) -> Self {
        Self {
            me_peak: peak_level(mic),
            me_rms: rms_level(mic),
            others_peak: peak_level(system),
            others_rms: rms_level(system),
        }
    }
}
