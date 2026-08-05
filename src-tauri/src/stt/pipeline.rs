use crate::domain::settings::{AppSettings, SttProvider};
use crate::domain::speaker::Speaker;
use crate::domain::transcript::{LiveTranscript, TranscriptSegment};
use crate::domain::vocabulary;
use crate::stt::local::LocalSttEngine;
use crate::stt::openrouter::OpenRouterStt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttChunkResult {
    pub speaker: Speaker,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// What this chunk cost, when a provider reported it. `None` for a local
    /// model, which is not the same as zero — a meeting that never left the
    /// machine shows no price at all.
    #[serde(default)]
    pub cost_nano_usd: Option<i64>,
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
        // The end of what this speaker last said, for the decoder to continue
        // from. Empty for the first utterance of a meeting, and for the
        // whole-recording passes, which have no "previous" to speak of.
        prompt: &str,
    ) -> Result<SttChunkResult, String> {
        if pcm.is_empty() || peak_abs(pcm) < 200 {
            return Ok(SttChunkResult {
                speaker,
                text: String::new(),
                cost_nano_usd: None,
                start_ms,
                end_ms: start_ms,
            });
        }
        let duration_ms = (pcm.len() as u64 * 1000) / sample_rate.max(1) as u64;
        let terms = vocabulary::normalise(&settings.hot_words);
        let (text, cost_nano_usd) = match settings.stt_provider {
            // whisper.cpp inference is CPU-bound and runs for seconds. Called
            // directly it parks a tokio worker for that whole time, and since the
            // UI polls every 1200ms the parked workers pile up until the runtime
            // has none left — which is what made live transcription unreliable.
            SttProvider::Local => {
                let engine = self.local.clone();
                let pcm = pcm.to_vec();
                let model = settings.local_stt_model.clone();
                let language = settings.language.clone();
                let backend = settings.compute_backend.clone();
                // The user's vocabulary, then the tail of what this speaker just
                // said. The tail is local-only — over the network it is tokens
                // per chunk for a continuity the cloud models already handle —
                // whereas the vocabulary is the whole point of a vocabulary and
                // goes to both.
                let prompt = vocabulary::initial_prompt(&terms, prompt);
                let text = tokio::task::spawn_blocking(move || {
                    engine.transcribe(&pcm, sample_rate, &model, &language, &backend, &prompt)
                })
                .await
                .map_err(|e| format!("transcription task failed: {e}"))??;
                (text, None)
            }
            SttProvider::OpenRouter => {
                // Audio is the most sensitive thing this application holds, so
                // the switch is checked before the key: a refusal that reads
                // "no API key" would send somebody looking for the wrong
                // problem.
                if let Some(refusal) = crate::domain::offline::refuse(
                    settings.offline_mode,
                    crate::domain::offline::Egress::CloudStt,
                ) {
                    return Err(refusal);
                }
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
                        &vocabulary::initial_prompt(&terms, ""),
                    )
                    .await?
            }
        };
        Ok(SttChunkResult {
            speaker,
            text,
            cost_nano_usd,
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
        // No continuation: this transcribes a whole recording in one call, so
        // there is no previous utterance to continue from. The user's vocabulary
        // still reaches the engine — `transcribe_channel` adds it.
        let (me, others) = tokio::join!(
            self.transcribe_channel(settings, Speaker::Me, &mic, sample_rate, start_ms, ""),
            self.transcribe_channel(
                settings,
                Speaker::Others,
                &system,
                sample_rate,
                start_ms,
                ""
            ),
        );
        let (me, others) = (me?, others?);
        // A chunk with no words can still have been billed — a cloud model
        // charges for the seconds of audio it listened to whether or not anyone
        // was speaking. Dropping it here lost the charge before any caller could
        // record it. `apply_stt_chunks` already ignores empty text, so keeping
        // it costs nothing downstream.
        if !me.text.is_empty() || me.cost_nano_usd.is_some() {
            out.push(me);
        }
        if !others.text.is_empty() || others.cost_nano_usd.is_some() {
            out.push(others);
        }
        Ok(out)
    }

    /// Transcribe a whole recording in one deliberate pass, keeping the engine's
    /// own line boundaries and clock.
    ///
    /// Not a longer `transcribe_dual`. That one asks for the recording as a
    /// single string, which is fine for an import that has no timestamps to lose;
    /// this replaces a transcript that had them, so it has to come back with a
    /// line and a time for each thing said.
    ///
    /// Local engine only, and the caller is what establishes that —
    /// `AppSettings::wants_final_stt_pass` is the gate, and the reason a cloud
    /// transcriber is not offered one is written there.
    ///
    /// Both channels, so Me and Others survive the replacement. They are
    /// transcribed independently and their timestamps both count from the start
    /// of the recording, which is what lets the two be interleaved afterwards.
    ///
    /// One after the other, unlike the chunked paths beside it. Concurrency buys
    /// nothing here — local inference serialises on the whisper context either
    /// way — and these are the largest buffers the application ever holds: a
    /// whole meeting per channel, each resampled to `f32` before it is decoded.
    /// Overlapping them would double that peak for no gain.
    ///
    /// The buffers arrive owned and are consumed. They come straight off the
    /// WAV, so borrowing would only mean copying them again to cross into the
    /// blocking task.
    pub async fn transcribe_whole_dual(
        &self,
        settings: &AppSettings,
        mic: Vec<i16>,
        system: Vec<i16>,
        sample_rate: u32,
    ) -> Result<Vec<SttChunkResult>, String> {
        let mut out = self
            .whole_channel(settings, Speaker::Me, mic, sample_rate)
            .await?;
        out.extend(
            self.whole_channel(settings, Speaker::Others, system, sample_rate)
                .await?,
        );
        Ok(out)
    }

    async fn whole_channel(
        &self,
        settings: &AppSettings,
        speaker: Speaker,
        pcm: Vec<i16>,
        sample_rate: u32,
    ) -> Result<Vec<SttChunkResult>, String> {
        // The same floor `transcribe_channel` uses. A solo recording carries a
        // system channel of zeros, and asking whisper to listen to an hour of
        // silence costs as much as asking it to listen to an hour of speech.
        if pcm.is_empty() || peak_abs(&pcm) < 200 {
            return Ok(Vec::new());
        }
        let engine = self.local.clone();
        let model = settings.local_stt_model.clone();
        let language = settings.language.clone();
        let backend = settings.compute_backend.clone();
        // Vocabulary and nothing else: there is no previous utterance to
        // continue from when the pass starts at the beginning of the meeting.
        let prompt = vocabulary::initial_prompt(&vocabulary::normalise(&settings.hot_words), "");
        // Off the runtime for the same reason the live path is: this is minutes
        // of CPU-bound inference, and run inline it would park a tokio worker
        // for all of it.
        let lines = tokio::task::spawn_blocking(move || {
            engine.transcribe_lines(&pcm, sample_rate, &model, &language, &backend, &prompt)
        })
        .await
        .map_err(|e| format!("transcription task failed: {e}"))??;
        Ok(lines
            .into_iter()
            .map(|l| SttChunkResult {
                speaker,
                text: l.text,
                // A local pass charges nothing, which is not the same as zero —
                // the meeting's price is left exactly as the live pass left it.
                cost_nano_usd: None,
                start_ms: l.start_ms,
                end_ms: l.end_ms,
            })
            .collect())
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
                cost_nano_usd: None,
                start_ms: 500,
                end_ms: 900,
            },
            SttChunkResult {
                speaker: Speaker::Me,
                text: "hello".into(),
                cost_nano_usd: None,
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
