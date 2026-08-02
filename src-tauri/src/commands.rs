use crate::audio::capture::{read_dual_wav, DualChannelRecorder};
use crate::audio::decode::decode_audio_file;
use crate::audio::devices::{list_audio_devices, AudioDevice};
use crate::db::{Database, KeyHome};
use crate::domain::capabilities::{detect_capabilities, CapabilityReport};
use crate::domain::chat::ChatMessage;
use crate::domain::export::{export_meeting, safe_file_stem, ExportFormat};
use crate::domain::gate::{can_start_recording_with, StartGate};
use crate::domain::i18n::{catalog, t, Locale};
use crate::domain::job::{
    MeetingEvent, MeetingPhase, MeetingProgress, MeetingRecord, MeetingStatus,
};
use crate::domain::search::SearchHit;
use crate::domain::settings::{AppSettings, LlmProvider, SttProvider};
use crate::domain::shortcut::ShortcutStatus;
use crate::domain::summary::{MeetingInsights, SummaryTemplate};
use crate::domain::title::{fallback_title, is_fallback_title, parse_title};
use crate::domain::transcript::LiveTranscript;
use crate::llm::service::LlmService;
use crate::models::{download_model_with_progress, list_models, DownloadProgress, ModelInfo};
use crate::paths::{ensure_app_dirs, recordings_dir};
use crate::stt::catalog::{
    default_llm_models, default_stt_models, fetch_openrouter_llm_models,
    fetch_openrouter_stt_models, OrModel,
};
use crate::stt::local::LocalSttEngine;
use crate::stt::pipeline::{apply_stt_chunks, SttService};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

pub struct AppState {
    pub db: Database,
    pub recorder: DualChannelRecorder,
    pub settings: Mutex<AppSettings>,
    pub live: Mutex<HashMap<String, LiveTranscript>>,
    pub active_meeting: Mutex<Option<String>>,
    /// Whether the API key is known to be in the OS keychain. False means a
    /// legacy row still holds it and the migration has yet to succeed.
    pub key_in_keychain: AtomicBool,
    /// Held for the duration of a transcription pass. `try_lock` gives the live
    /// poll its single-flight behaviour; `lock().await` lets stop wait for a pass
    /// that is already running instead of racing it to the transcript.
    pub stt_flight: tokio::sync::Mutex<()>,
    /// Held for the duration of a model download. One at a time, and enforced
    /// here rather than in the settings drawer: closing the drawer unmounts the
    /// component that knows a transfer is running, and two transfers of the same
    /// model append to one `.part` file.
    pub download_flight: tokio::sync::Mutex<()>,
    pub stt: SttService,
    pub llm: LlmService,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        ensure_app_dirs()?;
        let data = crate::paths::app_data_dir();
        let db = Database::open(&data)?;
        let mut settings = db.load_settings().unwrap_or_default();
        let (key, in_keychain) = load_or_migrate_api_key(&db);
        settings.openrouter_api_key = key;
        Ok(Self {
            db,
            recorder: DualChannelRecorder::new(),
            settings: Mutex::new(settings),
            live: Mutex::new(HashMap::new()),
            active_meeting: Mutex::new(None),
            key_in_keychain: AtomicBool::new(in_keychain),
            stt_flight: tokio::sync::Mutex::new(()),
            download_flight: tokio::sync::Mutex::new(()),
            stt: SttService::new(),
            llm: LlmService::new(),
        })
    }
}

/// Resolves the API key at startup, moving it out of the database on the first
/// run of a build that stores it in the keychain.
///
/// Returns the key and whether the keychain is the one holding it. The database
/// row is only cleared once the keychain has taken the value; if the keychain is
/// unavailable the key stays where it is and the app keeps working. Losing the
/// user's credential to be tidy would be the worse outcome.
fn load_or_migrate_api_key(db: &Database) -> (Option<String>, bool) {
    match db.legacy_api_key() {
        Ok(Some(legacy)) => match crate::secrets::store_openrouter_key(&legacy) {
            Ok(()) => {
                if let Err(e) = db.clear_legacy_api_key() {
                    tracing::warn!("key moved to the keychain but the old copy remains: {e}");
                }
                (Some(legacy), true)
            }
            Err(e) => {
                tracing::warn!("keychain unavailable, key stays in the database: {e}");
                (Some(legacy), false)
            }
        },
        Ok(None) => (crate::secrets::openrouter_key(), true),
        // We could not find out whether a legacy key is sitting in the row, so we
        // cannot claim the keychain is holding it. Saying "not migrated" costs one
        // redundant keychain write later; saying the opposite would let the next
        // save strip a row we never managed to read.
        Err(e) => {
            tracing::warn!("could not read stored settings: {e}");
            (crate::secrets::openrouter_key(), false)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderStatus {
    pub recording: bool,
    pub paused: bool,
    pub meeting_id: Option<String>,
    pub elapsed_ms: u64,
    pub levels: crate::audio::levels::ChannelLevels,
}

#[tauri::command]
pub fn get_settings(state: State<'_, Arc<AppState>>) -> AppSettings {
    state.settings.lock().public_view()
}

/// Blank and absent mean the same thing for a credential; comparing the raw
/// `Option<String>` would treat `Some("")` and `None` as a change.
fn normalised_key(key: &Option<String>) -> Option<String> {
    key.as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
}

/// The single write path for settings.
///
/// Both callers need the same three steps in the same order — restore a redacted
/// key, put the key in the keychain, then persist everything else — and the
/// keychain step is easy to forget when it is copied by hand.
fn persist_settings(state: &AppState, mut settings: AppSettings) -> Result<AppSettings, String> {
    // The front end only ever sees a redacted key, so a redacted value coming
    // back means "unchanged", not "set it to these characters".
    let (previous, previous_llm_model) = {
        let current = state.settings.lock();
        if let Some(k) = &settings.openrouter_api_key {
            if k.contains('…') || k == "****" {
                settings.openrouter_api_key = current.openrouter_api_key.clone();
            }
        }
        (
            normalised_key(&current.openrouter_api_key),
            current.openrouter_llm_model.clone(),
        )
    };
    settings.validate_models().map_err(|e| e.to_string())?;
    // The WebView is the trust boundary, so "it comes from our own front end" is
    // not a validation: a value outside this pair would be written to the row and
    // handed back to the window on every boot.
    if settings.theme != "light" && settings.theme != "dark" {
        return Err("invalid theme: expected `light` or `dark`".into());
    }
    // Only an actual change counts as a pick: every save comes through here, and a
    // save that touched the microphone must not reshuffle the model list.
    if settings.openrouter_llm_model != previous_llm_model {
        let picked = settings.openrouter_llm_model.clone();
        settings.remember_recent_llm_model(&picked);
    }
    // The key must be somewhere durable before the row is allowed to drop it, and
    // there are two candidate homes while a migration is outstanding. Decide which
    // one is holding it first, then write the row accordingly — `KeyHome` exists so
    // that decision cannot be skipped.
    let next = normalised_key(&settings.openrouter_api_key);
    let changed = next != previous;
    let mut home = KeyHome::Keychain;

    match &next {
        None if changed => crate::secrets::clear_openrouter_key()?,
        None => {}
        Some(key) => {
            // Write when the user changed it, and also when a previous run could
            // not migrate it — otherwise an unrelated save would strip the row
            // that is still the only durable copy.
            let already_safe = !changed && state.key_in_keychain.load(Ordering::Relaxed);
            if !already_safe {
                match crate::secrets::store_openrouter_key(key) {
                    Ok(()) => state.key_in_keychain.store(true, Ordering::Relaxed),
                    // The user typed this key, so tell them it will not be kept.
                    Err(e) if changed => return Err(e),
                    // They were doing something else entirely; keep the key where
                    // it already is rather than failing an unrelated action.
                    Err(e) => {
                        tracing::warn!("keychain still unavailable, key stays in the row: {e}");
                        home = KeyHome::KeepInRow;
                    }
                }
            }
        }
    }

    state.db.save_settings_with(&settings, home)?;
    *state.settings.lock() = settings.clone();
    Ok(settings)
}

#[tauri::command]
pub fn save_settings(
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    Ok(persist_settings(&state, settings)?.public_view())
}

#[tauri::command]
pub fn switch_stt_provider(
    state: State<'_, Arc<AppState>>,
    provider: String,
) -> Result<AppSettings, String> {
    let p = match provider.as_str() {
        "openrouter" => SttProvider::OpenRouter,
        _ => SttProvider::Local,
    };
    let mut s = state.settings.lock().clone();
    s.switch_stt(p).map_err(|e| e.to_string())?;
    Ok(persist_settings(&state, s)?.public_view())
}

#[tauri::command]
pub fn switch_llm_provider(
    state: State<'_, Arc<AppState>>,
    provider: String,
) -> Result<AppSettings, String> {
    let p = match provider.as_str() {
        "openrouter" => LlmProvider::OpenRouter,
        _ => LlmProvider::Local,
    };
    let mut s = state.settings.lock().clone();
    s.switch_llm(p).map_err(|e| e.to_string())?;
    Ok(persist_settings(&state, s)?.public_view())
}

#[tauri::command]
pub fn set_reasoning(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<AppSettings, String> {
    let mut s = state.settings.lock().clone();
    s.set_reasoning(enabled);
    Ok(persist_settings(&state, s)?.public_view())
}

#[tauri::command]
pub fn list_meetings(state: State<'_, Arc<AppState>>) -> Result<Vec<MeetingRecord>, String> {
    state.db.list_meetings()
}

#[tauri::command]
pub fn get_meeting(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Option<MeetingRecord>, String> {
    state.db.get_meeting(&id)
}

#[tauri::command]
pub fn get_transcript(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<LiveTranscript, String> {
    if let Some(t) = state.live.lock().get(&id) {
        return Ok(t.clone());
    }
    state.db.load_transcript(&id)
}

#[tauri::command]
pub fn delete_meeting(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state.db.delete_meeting(&id)
}

#[tauri::command]
pub fn search_meetings_cmd(
    state: State<'_, Arc<AppState>>,
    query: String,
) -> Result<Vec<SearchHit>, String> {
    state.db.search(&query, 50)
}

#[tauri::command]
pub fn recorder_status(state: State<'_, Arc<AppState>>) -> RecorderStatus {
    status_of(&state)
}

fn local_stt_ready(settings: &AppSettings) -> bool {
    LocalSttEngine::new().is_model_ready(&settings.local_stt_model)
}

/// Whether the artifact exists at all, regardless of verification.
fn local_stt_present(settings: &AppSettings) -> bool {
    list_models()
        .into_iter()
        .any(|m| m.id == settings.local_stt_model && m.present)
}

#[tauri::command]
pub fn can_record(state: State<'_, Arc<AppState>>) -> StartGate {
    let s = state.settings.lock().clone();
    can_start_recording_with(
        &s,
        local_stt_ready(&s),
        local_stt_present(&s),
        s.onboarding_complete,
    )
}

#[tauri::command]
pub fn list_audio_devices_cmd() -> Result<Vec<AudioDevice>, String> {
    list_audio_devices()
}

#[tauri::command]
pub fn get_i18n_catalog(locale: String) -> std::collections::HashMap<String, String> {
    catalog(Locale::from_code(&locale))
}

#[tauri::command]
pub fn translate_key(locale: String, key: String) -> String {
    t(Locale::from_code(&locale), &key)
}

#[tauri::command]
pub fn get_capabilities() -> CapabilityReport {
    detect_capabilities()
}

#[tauri::command]
pub async fn list_openrouter_stt_models(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<OrModel>, String> {
    let key = state
        .settings
        .lock()
        .openrouter_api_key
        .clone()
        .unwrap_or_default();
    if key.is_empty() || key.contains('…') {
        return Ok(default_stt_models());
    }
    fetch_openrouter_stt_models(&key).await
}

#[tauri::command]
pub async fn list_openrouter_llm_models(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<OrModel>, String> {
    let key = state
        .settings
        .lock()
        .openrouter_api_key
        .clone()
        .unwrap_or_default();
    if key.is_empty() || key.contains('…') {
        return Ok(default_llm_models());
    }
    fetch_openrouter_llm_models(&key).await
}

#[tauri::command]
pub fn complete_onboarding(
    state: State<'_, Arc<AppState>>,
    mut settings: AppSettings,
) -> Result<AppSettings, String> {
    settings.onboarding_complete = true;
    // Finishing onboarding is allowed even when the local model is still missing;
    // `can_record` is what actually blocks the recording later.
    Ok(persist_settings(&state, settings)?.public_view())
}

#[tauri::command]
pub fn start_recording(
    state: State<'_, Arc<AppState>>,
    title: Option<String>,
) -> Result<MeetingRecord, String> {
    if state.recorder.is_recording() {
        return Err("already recording".into());
    }
    let settings = state.settings.lock().clone();
    let gate = can_start_recording_with(
        &settings,
        local_stt_ready(&settings),
        local_stt_present(&settings),
        settings.onboarding_complete,
    );
    if !gate.allowed {
        return Err(gate
            .reason
            .unwrap_or_else(|| "Cannot start recording".into()));
    }
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    // Local time here while `created_at` above is UTC, on purpose: a title is a
    // label frozen at creation, and the sortable timestamp is the one that must
    // not move. `is_fallback_title` recognises what this writes, which is what
    // lets a generated title replace it and nothing else.
    let title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| fallback_title(chrono::Local::now()));
    let audio_path = recordings_dir().join(format!("{id}.wav"));
    state
        .recorder
        .start(
            audio_path.clone(),
            settings.mic_device_id.clone(),
            settings.system_device_id.clone(),
        )
        .map_err(|e| e.to_string())?;
    let mut status = MeetingStatus::Idle;
    status = status
        .transition(MeetingEvent::StartRecording)
        .map_err(|e| e.to_string())?;
    let meeting = MeetingRecord {
        id: id.clone(),
        title,
        status,
        created_at: now.clone(),
        updated_at: now,
        duration_ms: 0,
        audio_path: Some(audio_path.display().to_string()),
        transcript_text: String::new(),
        summary: None,
        action_items: None,
        key_points: None,
        project: None,
    };
    state.db.upsert_meeting(&meeting)?;
    state.live.lock().insert(id.clone(), LiveTranscript::new());
    *state.active_meeting.lock() = Some(id);
    Ok(meeting)
}

#[tauri::command]
pub fn pause_recording(state: State<'_, Arc<AppState>>) -> Result<RecorderStatus, String> {
    let id = state
        .active_meeting
        .lock()
        .clone()
        .ok_or_else(|| "no active meeting".to_string())?;
    if let Some(mut m) = state.db.get_meeting(&id)? {
        m.status = m
            .status
            .transition(MeetingEvent::Pause)
            .map_err(|e| e.to_string())?;
        m.updated_at = chrono::Utc::now().to_rfc3339();
        state.db.upsert_meeting(&m)?;
    }
    state.recorder.pause();
    Ok(status_of(&state))
}

#[tauri::command]
pub fn resume_recording(state: State<'_, Arc<AppState>>) -> Result<RecorderStatus, String> {
    let id = state
        .active_meeting
        .lock()
        .clone()
        .ok_or_else(|| "no active meeting".to_string())?;
    if let Some(mut m) = state.db.get_meeting(&id)? {
        m.status = m
            .status
            .transition(MeetingEvent::Resume)
            .map_err(|e| e.to_string())?;
        m.updated_at = chrono::Utc::now().to_rfc3339();
        state.db.upsert_meeting(&m)?;
    }
    state.recorder.resume();
    Ok(status_of(&state))
}

fn status_of(state: &AppState) -> RecorderStatus {
    RecorderStatus {
        recording: state.recorder.is_recording(),
        paused: state.recorder.is_paused(),
        meeting_id: state.active_meeting.lock().clone(),
        elapsed_ms: state.recorder.elapsed_ms(),
        levels: state.recorder.levels(),
    }
}

#[tauri::command]
pub async fn stop_recording(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<MeetingRecord, String> {
    let id = state
        .active_meeting
        .lock()
        .clone()
        .ok_or_else(|| "no active meeting".to_string())?;
    let path = state.recorder.stop().map_err(|e| e.to_string())?;
    let duration = state.recorder.elapsed_ms();
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting missing".to_string())?;
    meeting.status = meeting
        .status
        .transition(MeetingEvent::StopRecording)
        .map_err(|e| e.to_string())?;
    meeting.duration_ms = duration;
    meeting.audio_path = Some(path.display().to_string());
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    *state.active_meeting.lock() = None;
    // First point where the command can no longer fail early: the recording is
    // on disk and the row knows where. Announcing a phase before this would
    // leave the window on a phase for a command that then returned an error.
    let _ = app.emit(
        "meeting://progress",
        &MeetingProgress::new(&id, MeetingPhase::Saving),
    );

    // Wait for a live poll that is still transcribing before reading the
    // transcript. Without this, stopping mid-chunk reads a transcript that is
    // missing the tail, persists it, marks the meeting ready — and the poll then
    // writes its result into the in-memory map only. Reopening the meeting shows
    // the transcript with the last chunk gone.
    let _flight = state.stt_flight.lock().await;

    let settings = state.settings.lock().clone();
    let mut live = state.live.lock().get(&id).cloned().unwrap_or_default();
    let _ = app.emit(
        "meeting://progress",
        &MeetingProgress::new(&id, MeetingPhase::Transcribing),
    );

    if live.segments().is_empty() {
        // Nothing was transcribed live — cloud STT down, or a recording short
        // enough that no poll ever ran. Transcribe the whole saved WAV.
        if let Ok((mic, sys, sr)) = read_dual_wav(&path) {
            if let Ok(chunks) = state
                .stt
                .transcribe_dual(&settings, &mic, &sys, sr, 0)
                .await
            {
                apply_stt_chunks(&mut live, &chunks);
            }
        }
    } else {
        // Live transcription only ever consumed what the last poll drained, so
        // everything spoken between that drain and the stop is still sitting in
        // the buffer. Skipping it — which is what happened whenever any live
        // segment existed — silently dropped the end of every meeting.
        let (mic, sys, sr) = state.recorder.drain_chunks();
        if !mic.is_empty() || !sys.is_empty() {
            let tail_ms = duration
                .saturating_sub((mic.len().max(sys.len()) as u64 * 1000) / sr.max(1) as u64);
            match state
                .stt
                .transcribe_dual(&settings, &mic, &sys, sr, tail_ms)
                .await
            {
                Ok(chunks) => apply_stt_chunks(&mut live, &chunks),
                Err(e) => tracing::warn!("final chunk could not be transcribed: {e}"),
            }
        }
    }
    state.live.lock().insert(id.clone(), live.clone());
    state.db.save_transcript(&id, &live)?;
    meeting.transcript_text = live.plain_text();
    meeting.status = meeting
        .status
        .transition(MeetingEvent::TranscribeDone)
        .map_err(|e| e.to_string())?;
    state.db.upsert_meeting(&meeting)?;

    let mut done = MeetingProgress::new(&id, MeetingPhase::Ready);
    if settings.auto_summarize && !meeting.transcript_text.is_empty() {
        let _ = app.emit(
            "meeting://progress",
            &MeetingProgress::new(&id, MeetingPhase::Summarizing),
        );
        match state
            .llm
            .summarize(
                &settings,
                &meeting.transcript_text,
                SummaryTemplate::General,
            )
            .await
        {
            Ok(insights) => {
                state.db.save_insights(&id, &insights)?;
                meeting.summary = Some(insights.summary.clone());
                meeting.action_items = Some(insights.action_items_text());
                meeting.key_points = Some(insights.key_points_text());
                meeting.status = MeetingStatus::Ready;
                name_meeting(&state, &settings, &mut meeting, &insights.summary).await;
                meeting.updated_at = chrono::Utc::now().to_rfc3339();
                state.db.upsert_meeting(&meeting)?;
            }
            // The transcript is already saved and the meeting is already Ready,
            // so propagating this told the user their recording was lost when
            // only the summary was. Report the summary, keep the meeting.
            Err(e) => {
                tracing::warn!("auto-summary failed: {e}");
                done = MeetingProgress::summary_failed(&id, &e);
            }
        }
    }

    // One terminal phase, emitted once: a `ready` after a `summary_failed` would
    // supersede it on the single channel and the failure would never be seen.
    let _ = app.emit("meeting://progress", &done);
    let _ = app.emit("meeting://ready", &meeting);
    Ok(meeting)
}

#[tauri::command]
pub async fn poll_live_stt(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<LiveTranscript, String> {
    let id = state
        .active_meeting
        .lock()
        .clone()
        .ok_or_else(|| "no active meeting".to_string())?;
    if !state.recorder.is_recording() || state.recorder.is_paused() {
        return Ok(state.live.lock().get(&id).cloned().unwrap_or_default());
    }
    // Single-flight. The UI polls every 1200ms and a chunk can take longer than
    // that to transcribe, so without this the next poll drains a second slice of
    // audio while the first is still running. Both then timestamp their slice from
    // whatever `elapsed_ms` reads at drain time, and the transcript comes out in
    // the wrong order. Overlapping polls now just return what is already there;
    // the audio stays in the buffer for the next turn.
    let Ok(_flight) = state.stt_flight.try_lock() else {
        return Ok(state.live.lock().get(&id).cloned().unwrap_or_default());
    };
    let (mic, sys, sr) = state.recorder.drain_chunks();
    if mic.is_empty() && sys.is_empty() {
        return Ok(state.live.lock().get(&id).cloned().unwrap_or_default());
    }
    let start_ms = state
        .recorder
        .elapsed_ms()
        .saturating_sub((mic.len().max(sys.len()) as u64 * 1000) / sr.max(1) as u64);
    let settings = state.settings.lock().clone();
    let chunks = state
        .stt
        .transcribe_dual(&settings, &mic, &sys, sr, start_ms)
        .await?;
    let mut guard = state.live.lock();
    let t = guard.entry(id.clone()).or_default();
    apply_stt_chunks(t, &chunks);
    let snapshot = t.clone();
    drop(guard);
    let _ = app.emit("transcript://append", &snapshot);
    Ok(snapshot)
}

/// Replace the date label with a title read out of the meeting itself.
///
/// Every failure is silent by design. A generated title is a guess: if the model
/// is missing, refuses or answers with nothing usable, the meeting keeps the
/// label it was born with and the user is told nothing, because there is nothing
/// they could do about it. Anything that is *not* the date label — a name the
/// user typed, an import's file stem — is never touched.
///
/// Sends the summary and an excerpt of the transcript to whichever provider the
/// user already chose for summarisation. Under OpenRouter that is a second
/// billed call and the excerpt leaves the machine again; under the local model
/// nothing leaves at all.
async fn name_meeting(
    state: &AppState,
    settings: &AppSettings,
    meeting: &mut MeetingRecord,
    summary: &str,
) {
    if !is_fallback_title(&meeting.title) {
        return;
    }
    match state
        .llm
        .title(settings, summary, &meeting.transcript_text)
        .await
    {
        Ok(raw) => match parse_title(&raw) {
            Some(title) => meeting.title = title,
            None => tracing::warn!("the model answered with no usable title"),
        },
        Err(e) => tracing::warn!("title generation failed: {e}"),
    }
}

#[tauri::command]
pub async fn summarize_meeting(
    state: State<'_, Arc<AppState>>,
    id: String,
    template: Option<String>,
) -> Result<MeetingInsights, String> {
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let tpl = SummaryTemplate::from_id(template.as_deref().unwrap_or("general"));
    let settings = state.settings.lock().clone();
    let mut m = meeting;
    m.status = m
        .status
        .transition(MeetingEvent::StartSummarize)
        .unwrap_or(MeetingStatus::Summarizing);
    state.db.upsert_meeting(&m)?;
    let insights = state
        .llm
        .summarize(&settings, &m.transcript_text, tpl)
        .await?;
    state.db.save_insights(&id, &insights)?;
    m.status = MeetingStatus::Ready;
    m.summary = Some(insights.summary.clone());
    m.action_items = Some(insights.action_items_text());
    m.key_points = Some(insights.key_points_text());
    // Re-summarising is also the way an old meeting still carrying its date
    // label gets a real name.
    name_meeting(&state, &settings, &mut m, &insights.summary).await;
    m.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&m)?;
    Ok(insights)
}

#[tauri::command]
pub async fn chat_meeting(
    state: State<'_, Arc<AppState>>,
    id: String,
    question: String,
) -> Result<ChatMessage, String> {
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let history = state.db.list_chat(&id)?;
    let settings = state.settings.lock().clone();
    state.db.add_chat(&id, "user", &question)?;
    let answer = state
        .llm
        .chat(
            &settings,
            &meeting.title,
            &meeting.transcript_text,
            meeting.summary.as_deref(),
            &history,
            &question,
        )
        .await?;
    state.db.add_chat(&id, "assistant", &answer)?;
    Ok(ChatMessage {
        role: "assistant".into(),
        content: answer,
    })
}

#[tauri::command]
pub fn list_chat(state: State<'_, Arc<AppState>>, id: String) -> Result<Vec<ChatMessage>, String> {
    state.db.list_chat(&id)
}

#[tauri::command]
pub async fn import_audio(
    state: State<'_, Arc<AppState>>,
    path: String,
    title: Option<String>,
) -> Result<MeetingRecord, String> {
    let path = PathBuf::from(path);
    // Multi-format: wav/mp3/m4a/ogg/flac/webm via decode layer.
    // The length ceiling lives in the decoder, where it can stop before the
    // allocation happens — see MAX_DECODED_SAMPLES.
    let (pcm, sr) = decode_audio_file(&path).map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported meeting".into())
    });
    // Persist a dual-channel WAV copy under recordings for retranscription
    let wav_path = recordings_dir().join(format!("{id}.wav"));
    crate::audio::capture::write_dual_wav(&wav_path, sr, &pcm, &[]).map_err(|e| e.to_string())?;
    let mut meeting = MeetingRecord {
        id: id.clone(),
        title,
        status: MeetingStatus::Transcribing,
        created_at: now.clone(),
        updated_at: now,
        duration_ms: (pcm.len() as u64 * 1000) / sr.max(1) as u64,
        audio_path: Some(wav_path.display().to_string()),
        transcript_text: String::new(),
        summary: None,
        action_items: None,
        key_points: None,
        project: None,
    };
    state.db.upsert_meeting(&meeting)?;
    let settings = state.settings.lock().clone();
    // Mono import: Me channel only (no dual split in source file)
    let chunks = state
        .stt
        .transcribe_dual(&settings, &pcm, &[], sr, 0)
        .await?;
    let mut t = LiveTranscript::new();
    apply_stt_chunks(&mut t, &chunks);
    state.db.save_transcript(&id, &t)?;
    meeting.transcript_text = t.plain_text();
    meeting.status = MeetingStatus::Ready;
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    Ok(meeting)
}

#[tauri::command]
pub async fn retranscribe(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<MeetingRecord, String> {
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let audio = meeting
        .audio_path
        .clone()
        .ok_or_else(|| "no audio for meeting".to_string())?;
    let audio_path = std::path::Path::new(&audio);
    let (mic, sys, sr) = if audio_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false)
    {
        read_dual_wav(audio_path).map_err(|e| e.to_string())?
    } else {
        let (pcm, sr) = decode_audio_file(audio_path).map_err(|e| e.to_string())?;
        (pcm, Vec::new(), sr)
    };
    meeting.status = MeetingStatus::Transcribing;
    state.db.upsert_meeting(&meeting)?;
    let settings = state.settings.lock().clone();
    let chunks = state
        .stt
        .transcribe_dual(&settings, &mic, &sys, sr, 0)
        .await?;
    let mut t = LiveTranscript::new();
    apply_stt_chunks(&mut t, &chunks);
    state.db.save_transcript(&id, &t)?;
    meeting.transcript_text = t.plain_text();
    meeting.status = MeetingStatus::Ready;
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    Ok(meeting)
}

#[tauri::command]
pub fn export_meeting_cmd(
    state: State<'_, Arc<AppState>>,
    id: String,
    path: String,
    format: String,
) -> Result<String, String> {
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let transcript = state.db.load_transcript(&id)?;
    let insights = MeetingInsights {
        summary: meeting.summary.clone().unwrap_or_default(),
        key_points: meeting
            .key_points
            .clone()
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim_start_matches('-').trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        action_items: meeting
            .action_items
            .clone()
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim_start_matches('-').trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    };
    let fmt = ExportFormat::from_ext(&format).ok_or_else(|| "unsupported format".to_string())?;
    export_meeting(
        std::path::Path::new(&path),
        fmt,
        &meeting.title,
        &transcript,
        Some(&insights),
    )
}

#[tauri::command]
pub fn list_models_cmd() -> Vec<ModelInfo> {
    list_models()
}

/// Downloads a catalog model by id. There is deliberately no URL parameter: the
/// artifact is fed to whisper.cpp / llama.cpp, so letting the WebView choose where
/// the bytes come from would hand an attacker the input to a C++ parser.
#[tauri::command]
pub async fn download_model_cmd(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    model_id: String,
) -> Result<String, String> {
    // `try_lock`, not `lock().await`: a queued second download is a click the user
    // has forgotten about by the time it starts. Refusing is the honest answer, and
    // the drawer's disabled buttons make this unreachable in the ordinary case —
    // this catches the one they cannot cover, where the drawer was closed and
    // reopened while the transfer kept running.
    let Ok(_flight) = state.download_flight.try_lock() else {
        return Err("another model is already downloading".into());
    };
    let path = download_model_with_progress(&model_id, move |p: DownloadProgress| {
        // A dropped frame is how the freeze looked from the UI: the queue to the
        // window thread saturates, the emit fails, and the percentage stops moving
        // while bytes keep arriving. The pacer should make this unreachable — if it
        // ever fires, that is the thing to look at.
        if let Err(e) = app.emit("models://download-progress", &p) {
            tracing::warn!("progress event dropped: {e}");
        }
    })
    .await?;
    Ok(path.display().to_string())
}

/// Reports the updater config for UI/tests without network.
///
/// Everything is read from `tauri.conf.json` at compile time rather than restated
/// here. The previous version hardcoded `pubkey_configured: true` while the shipped
/// key was a development placeholder — claiming a signature guarantee the build did
/// not have. A hand-kept mirror of a config file drifts; a derived one cannot.
#[tauri::command]
pub fn check_updates_config() -> serde_json::Value {
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).unwrap_or_default();
    let updater = conf.pointer("/plugins/updater");
    serde_json::json!({
        "active": updater.is_some(),
        "endpoints": updater
            .and_then(|u| u.get("endpoints"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "targets": ["windows", "macos", "linux"],
        "pubkey_configured": updater_pubkey_is_real(&conf)
    })
}

const TAURI_CONF: &str = include_str!("../tauri.conf.json");

/// True only when the updater public key is structurally a minisign public key.
///
/// Checking for the placeholder's wording would be enough to catch today's value
/// and nothing else: any other base64 string — `dGVzdA==` decodes to `test` —
/// would pass while still being unusable for signature verification, putting the
/// command right back to claiming a guarantee the build does not have.
///
/// A minisign public key file is an untrusted-comment line followed by a base64
/// line carrying 42 bytes: a two-byte algorithm tag, an eight-byte key id and the
/// 32-byte key. The development placeholder has no such line at all.
fn updater_pubkey_is_real(conf: &serde_json::Value) -> bool {
    use base64::Engine as _;
    let Some(pubkey) = conf
        .pointer("/plugins/updater/pubkey")
        .and_then(|k| k.as_str())
    else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(pubkey.trim()) else {
        return false;
    };
    String::from_utf8_lossy(&decoded)
        .lines()
        .any(is_minisign_key_line)
}

fn is_minisign_key_line(line: &str) -> bool {
    use base64::Engine as _;
    let line = line.trim();
    // "Ed" is the only signature algorithm minisign emits for public keys.
    base64::engine::general_purpose::STANDARD
        .decode(line)
        .map(|raw| raw.len() == 42 && raw.starts_with(b"Ed"))
        .unwrap_or(false)
}

/// Whether the OS granted the global accelerator, and which one it is.
///
/// Registration happens once during setup; this only reads the result, so the
/// window can render the key it will actually get instead of a hardcoded string
/// derived from nothing the backend reports.
#[tauri::command]
pub fn shortcut_status(status: State<'_, ShortcutStatus>) -> ShortcutStatus {
    status.inner().clone()
}

/// Rename a meeting.
///
/// The typed name goes through the same `parse_title` as the model's answer —
/// the WebView is the trust boundary, and a title reaches a filename, a PDF
/// header and an FTS index. `upsert_meeting` re-indexes search in the same
/// transaction, so the new name is findable immediately.
#[tauri::command]
pub fn rename_meeting(
    state: State<'_, Arc<AppState>>,
    id: String,
    title: String,
) -> Result<MeetingRecord, String> {
    let title = parse_title(&title).ok_or_else(|| "a meeting needs a name".to_string())?;
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    meeting.title = title;
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    Ok(meeting)
}

/// The filename the export dialog should open with.
///
/// The stem is the meeting's own title, made safe for the filesystem here rather
/// than in the front end: the rules are Windows' and a sanitiser written in
/// TypeScript could not be covered by any test this repo runs.
#[tauri::command]
pub fn suggested_export_name(
    state: State<'_, Arc<AppState>>,
    id: String,
    format: String,
) -> Result<String, String> {
    let fmt = ExportFormat::from_ext(&format).ok_or_else(|| "unsupported format".to_string())?;
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    Ok(format!(
        "{}.{}",
        safe_file_stem(&meeting.title),
        fmt.extension()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn conf_with_pubkey(pubkey: &str) -> serde_json::Value {
        serde_json::json!({ "plugins": { "updater": { "pubkey": pubkey } } })
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn the_shipped_key_is_a_real_signing_key() {
        let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).unwrap();
        assert!(
            updater_pubkey_is_real(&conf),
            "tauri.conf.json no longer carries a usable minisign public key; without one \
             the updater cannot verify a release, and every install would accept whatever \
             the endpoint serves"
        );
    }

    #[test]
    fn an_arbitrary_base64_string_is_not_a_key() {
        // Decodes to "test": no minisign key line, so it cannot verify anything.
        assert!(!updater_pubkey_is_real(&conf_with_pubkey("dGVzdA==")));
        assert!(!updater_pubkey_is_real(&conf_with_pubkey("")));
        assert!(!updater_pubkey_is_real(&conf_with_pubkey(
            "not base64 at all"
        )));
        assert!(!updater_pubkey_is_real(&serde_json::json!({})));
    }

    #[test]
    fn a_structurally_valid_minisign_key_is_accepted() {
        let mut raw = Vec::from(*b"Ed");
        raw.extend_from_slice(&[7u8; 8]); // key id
        raw.extend_from_slice(&[9u8; 32]); // key
        let file = format!("untrusted comment: minisign public key\n{}\n", b64(&raw));
        assert!(updater_pubkey_is_real(&conf_with_pubkey(&b64(
            file.as_bytes()
        ))));
    }

    #[test]
    fn a_key_line_of_the_wrong_length_is_rejected() {
        let raw = Vec::from(*b"Ed");
        let file = format!("untrusted comment: truncated\n{}\n", b64(&raw));
        assert!(!updater_pubkey_is_real(&conf_with_pubkey(&b64(
            file.as_bytes()
        ))));
    }
}
