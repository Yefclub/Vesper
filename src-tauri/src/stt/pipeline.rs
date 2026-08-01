use crate::domain::settings::{AppSettings, SttProvider};
use crate::domain::speaker::Speaker;
use crate::domain::transcript::{LiveTranscript, TranscriptSegment};
use crate::stt::local::LocalSttEngine;
use crate::stt::openrouter::OpenRouterStt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttChunkResult {
    pub speaker: Speaker,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Pure merge of STT chunk results into a live transcript (testable without engines).
pub fn apply_stt_chunks(transcript: &mut LiveTranscript, chunks: &[SttChunkResult]) {
    for c in chunks {
        if c.text.trim().is_empty() {
            continue;
        }
        transcript.append(TranscriptSegment::new(
            c.speaker,
            c.text.clone(),
            c.start_ms,
            c.end_ms,
        ));
    }
}

/// Split dual-channel PCM into per-speaker frames for independent transcription.
pub fn split_dual_for_stt(mic: &[i16], system: &[i16]) -> (Vec<i16>, Vec<i16>) {
    (mic.to_vec(), system.to_vec())
}

pub struct SttService {
    local: LocalSttEngine,
    remote: OpenRouterStt,
}

impl SttService {
    pub fn new() -> Self {
        Self {
            local: LocalSttEngine::new(),
            remote: OpenRouterStt::new(),
        }
    }

    pub async fn transcribe_channel(
        &self,
        settings: &AppSettings,
        speaker: Speaker,
        pcm: &[i16],
        sample_rate: u32,
        start_ms: u64,
    ) -> Result<SttChunkResult, String> {
        if pcm.is_empty() || peak_abs(pcm) < 200 {
            return Ok(SttChunkResult {
                speaker,
                text: String::new(),
                start_ms,
                end_ms: start_ms,
            });
        }
        let duration_ms = (pcm.len() as u64 * 1000) / sample_rate.max(1) as u64;
        let text = match settings.stt_provider {
            // whisper.cpp inference is CPU-bound and runs for seconds. Called
            // directly it parks a tokio worker for that whole time, and since the
            // UI polls every 1200ms the parked workers pile up until the runtime
            // has none left — which is what made live transcription unreliable.
            SttProvider::Local => {
                let engine = self.local.clone();
                let pcm = pcm.to_vec();
                let model = settings.local_stt_model.clone();
                let language = settings.language.clone();
                tokio::task::spawn_blocking(move || {
                    engine.transcribe(&pcm, sample_rate, &model, &language)
                })
                .await
                .map_err(|e| format!("transcription task failed: {e}"))??
            }
            SttProvider::OpenRouter => {
                settings
                    .require_openrouter_key()
                    .map_err(|e| e.to_string())?;
                self.remote
                    .transcribe(
                        pcm,
                        sample_rate,
                        settings.openrouter_api_key.as_deref().unwrap_or(""),
                        &settings.openrouter_stt_model,
                        &settings.language,
                    )
                    .await?
            }
        };
        Ok(SttChunkResult {
            speaker,
            text,
            start_ms,
            end_ms: start_ms + duration_ms,
        })
    }

    pub async fn transcribe_dual(
        &self,
        settings: &AppSettings,
        mic: &[i16],
        system: &[i16],
        sample_rate: u32,
        start_ms: u64,
    ) -> Result<Vec<SttChunkResult>, String> {
        let (mic, system) = split_dual_for_stt(mic, system);
        let mut out = Vec::new();
        // The two channels are independent, so waiting for one before starting the
        // other doubled the latency of every chunk for no reason. Local inference
        // still serialises on the whisper context, but it does so on blocking
        // threads instead of holding the caller.
        let (me, others) = tokio::join!(
            self.transcribe_channel(settings, Speaker::Me, &mic, sample_rate, start_ms),
            self.transcribe_channel(settings, Speaker::Others, &system, sample_rate, start_ms),
        );
        let (me, others) = (me?, others?);
        if !me.text.is_empty() {
            out.push(me);
        }
        if !others.text.is_empty() {
            out.push(others);
        }
        Ok(out)
    }
}

impl Default for SttService {
    fn default() -> Self {
        Self::new()
    }
}

fn peak_abs(pcm: &[i16]) -> i16 {
    pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_chunks_orders_speakers() {
        let mut t = LiveTranscript::new();
        let chunks = vec![
            SttChunkResult {
                speaker: Speaker::Others,
                text: "world".into(),
                start_ms: 500,
                end_ms: 900,
            },
            SttChunkResult {
                speaker: Speaker::Me,
                text: "hello".into(),
                start_ms: 0,
                end_ms: 400,
            },
        ];
        apply_stt_chunks(&mut t, &chunks);
        assert_eq!(t.segments()[0].text, "hello");
        assert_eq!(t.segments()[0].speaker, Speaker::Me);
        assert_eq!(t.segments()[1].text, "world");
    }

    #[test]
    fn split_dual_preserves_channels() {
        let (a, b) = split_dual_for_stt(&[1, 2], &[3, 4]);
        assert_eq!(a, vec![1, 2]);
        assert_eq!(b, vec![3, 4]);
    }
}
