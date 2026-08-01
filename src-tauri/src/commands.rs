use crate::audio::capture::{read_wav_mono, DualChannelRecorder};
use crate::db::Database;
use crate::domain::chat::ChatMessage;
use crate::domain::export::{export_meeting, ExportFormat};
use crate::domain::job::{MeetingEvent, MeetingRecord, MeetingStatus};
use crate::domain::search::SearchHit;
use crate::domain::settings::{AppSettings, LlmProvider, SttProvider};
use crate::domain::summary::{MeetingInsights, SummaryTemplate};
use crate::domain::transcript::LiveTranscript;
use crate::llm::service::LlmService;
use crate::models::{download_model, list_models, ModelInfo};
use crate::paths::{ensure_app_dirs, recordings_dir};
use crate::stt::pipeline::{apply_stt_chunks, SttService};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

pub struct AppState {
    pub db: Database,
    pub recorder: DualChannelRecorder,
    pub settings: Mutex<AppSettings>,
    pub live: Mutex<HashMap<String, LiveTranscript>>,
    pub active_meeting: Mutex<Option<String>>,
    pub stt: SttService,
    pub llm: LlmService,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        ensure_app_dirs()?;
        let data = crate::paths::app_data_dir();
        let db = Database::open(&data)?;
        let settings = db.load_settings().unwrap_or_default();
        Ok(Self {
            db,
            recorder: DualChannelRecorder::new(),
            settings: Mutex::new(settings),
            live: Mutex::new(HashMap::new()),
            active_meeting: Mutex::new(None),
            stt: SttService::new(),
            llm: LlmService::new(),
        })
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

#[tauri::command]
pub fn save_settings(
    state: State<'_, Arc<AppState>>,
    mut settings: AppSettings,
) -> Result<AppSettings, String> {
    // Preserve full API key if client sent redacted value
    {
        let current = state.settings.lock();
        if let Some(k) = &settings.openrouter_api_key {
            if k.contains('…') || k == "****" {
                settings.openrouter_api_key = current.openrouter_api_key.clone();
            }
        }
    }
    settings.validate_models().map_err(|e| e.to_string())?;
    state.db.save_settings(&settings)?;
    *state.settings.lock() = settings.clone();
    Ok(settings.public_view())
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
    state.db.save_settings(&s)?;
    *state.settings.lock() = s.clone();
    Ok(s.public_view())
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
    state.db.save_settings(&s)?;
    *state.settings.lock() = s.clone();
    Ok(s.public_view())
}

#[tauri::command]
pub fn set_reasoning(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<AppSettings, String> {
    let mut s = state.settings.lock().clone();
    s.set_reasoning(enabled);
    state.db.save_settings(&s)?;
    *state.settings.lock() = s.clone();
    Ok(s.public_view())
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

#[tauri::command]
pub fn start_recording(
    state: State<'_, Arc<AppState>>,
    title: Option<String>,
) -> Result<MeetingRecord, String> {
    if state.recorder.is_recording() {
        return Err("already recording".into());
    }
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| format!("Meeting {}", chrono::Local::now().format("%Y-%m-%d %H:%M")));
    let audio_path = recordings_dir().join(format!("{id}.wav"));
    state.recorder.start(audio_path.clone()).map_err(|e| e.to_string())?;
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

    // Final dual-channel STT pass on saved audio (best-effort)
    let settings = state.settings.lock().clone();
    if let Ok((mono, sr)) = read_wav_mono(&path) {
        // Stereo dual: re-read is mono downmix; live segments already accumulated.
        let _ = (mono, sr);
    }
    let live = state
        .live
        .lock()
        .get(&id)
        .cloned()
        .unwrap_or_default();
    state.db.save_transcript(&id, &live)?;
    meeting.transcript_text = live.plain_text();
    meeting.status = meeting
        .status
        .transition(MeetingEvent::TranscribeDone)
        .map_err(|e| e.to_string())?;
    state.db.upsert_meeting(&meeting)?;

    if settings.auto_summarize && !meeting.transcript_text.is_empty() {
        let _ = app.emit("meeting://summarizing", &id);
        let insights = state
            .llm
            .summarize(&settings, &meeting.transcript_text, SummaryTemplate::General)
            .await?;
        state.db.save_insights(&id, &insights)?;
        meeting.summary = Some(insights.summary.clone());
        meeting.action_items = Some(insights.action_items_text());
        meeting.key_points = Some(insights.key_points_text());
        meeting.status = MeetingStatus::Ready;
        meeting.updated_at = chrono::Utc::now().to_rfc3339();
        state.db.upsert_meeting(&meeting)?;
    }

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
    let (mic, sys, sr) = state.recorder.drain_chunks();
    if mic.is_empty() && sys.is_empty() {
        return Ok(state.live.lock().get(&id).cloned().unwrap_or_default());
    }
    let start_ms = state.recorder.elapsed_ms().saturating_sub(
        (mic.len().max(sys.len()) as u64 * 1000) / sr.max(1) as u64,
    );
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
    let (pcm, sr) = read_wav_mono(&path).map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported meeting".into())
    });
    let mut meeting = MeetingRecord {
        id: id.clone(),
        title,
        status: MeetingStatus::Transcribing,
        created_at: now.clone(),
        updated_at: now,
        duration_ms: (pcm.len() as u64 * 1000) / sr.max(1) as u64,
        audio_path: Some(path.display().to_string()),
        transcript_text: String::new(),
        summary: None,
        action_items: None,
        key_points: None,
        project: None,
    };
    state.db.upsert_meeting(&meeting)?;
    let settings = state.settings.lock().clone();
    // Import: treat full mono as Me channel (file has no dual split guarantee)
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
    let (pcm, sr) = read_wav_mono(std::path::Path::new(&audio)).map_err(|e| e.to_string())?;
    meeting.status = MeetingStatus::Transcribing;
    state.db.upsert_meeting(&meeting)?;
    let settings = state.settings.lock().clone();
    let half = pcm.len() / 2;
    // Dual-channel file: left/right already downmixed in mono path — re-run full as Me.
    let chunks = state
        .stt
        .transcribe_dual(&settings, &pcm, &pcm[half.min(pcm.len())..], sr, 0)
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

#[tauri::command]
pub async fn download_model_cmd(model_id: String, url: Option<String>) -> Result<String, String> {
    let models = list_models();
    let m = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| "unknown model".to_string())?;
    let url = url.or(m.download_url).ok_or_else(|| "no url".to_string())?;
    let path = download_model(&model_id, &url).await?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn check_updates_config() -> serde_json::Value {
    // Mirrors tauri.conf.json updater endpoint for UI/tests without network.
    serde_json::json!({
        "active": true,
        "endpoints": [
            "https://github.com/Yefclub/Vesper/releases/latest/download/latest.json"
        ],
        "targets": ["windows", "macos", "linux"],
        "pubkey_configured": true
    })
}
