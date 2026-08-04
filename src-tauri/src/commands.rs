use crate::audio::capture::{read_dual_wav, DualChannelRecorder};
use crate::audio::decode::decode_audio_file;
use crate::audio::devices::{list_audio_devices, AudioDevice};
use crate::db::{Database, KeyHome};
use crate::domain::actions::ActionItem;
use crate::domain::capabilities::{detect_capabilities, CapabilityReport};
use crate::domain::chat::ChatMessage;
use crate::domain::export::{export_meeting, safe_file_stem, ExportFormat};
use crate::domain::gate::{can_start_recording_with, StartGate};
use crate::domain::i18n::{catalog, t, Locale};
use crate::domain::job::{
    MeetingEvent, MeetingPhase, MeetingProgress, MeetingRecord, MeetingStatus,
};
use crate::domain::overlay::{dock_right_center, overlay_visible, COLLAPSED, EXPANDED};
use crate::domain::refine::{build_refine_prompt, parse_refined_list, Section, SummaryVersion};
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
use crate::stt::pipeline::{apply_stt_chunks, SttChunkResult, SttService};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
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
    /// Bumped by every start and stop, so only the newest live-STT ticker keeps
    /// draining audio. Two tickers would split each window between them and run
    /// concurrent transcriptions over halves of the same speech.
    pub live_stt_generation: AtomicU64,
    /// Held across a refinement's whole read, model call and write.
    ///
    /// A version is the *whole* insight set, so two refinements that each read
    /// the meeting before either wrote would each build a version from a stale
    /// snapshot, and the second to land would carry the other's section from
    /// before it was improved. The window already allows one at a time; this is
    /// what makes it true.
    pub refine_flight: tokio::sync::Mutex<()>,
    /// Which meetings have had at least one live transcription pass complete.
    ///
    /// Not the same question as "does the transcript have segments": a paid pass
    /// can answer with no words and still have been billed, and treating that as
    /// "no live transcription happened" sends the whole recording through the
    /// provider a second time at stop — charging twice for the same audio.
    ///
    /// Keyed by meeting rather than counted globally: stopping one recording
    /// while its pass is still in flight and immediately starting another let
    /// the first one's completion answer for the second, and the second then
    /// skipped a fallback it needed.
    pub live_stt_passes: Mutex<HashSet<String>>,
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
            live_stt_generation: AtomicU64::new(0),
            refine_flight: tokio::sync::Mutex::new(()),
            live_stt_passes: Mutex::new(HashSet::new()),
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
/// Which local weights the settings are asking for, if any.
///
/// `None` for a cloud provider: nothing local is wanted at all, which is a
/// different state from wanting a different local model and has to compare
/// unequal to it.
fn wanted_local_weights(settings: &AppSettings) -> Option<(String, String)> {
    match settings.llm_provider {
        LlmProvider::Local => Some((
            settings.local_llm_model.clone(),
            settings.compute_backend.clone(),
        )),
        _ => None,
    }
}

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
    // Which weights are wanted, before the save decides otherwise, and whether
    // summaries were being produced without being asked for. Both compared after
    // the write so a change frees what the old answer was holding.
    let (previously_wanted, previously_automatic) = {
        let current = state.settings.lock();
        (wanted_local_weights(&current), current.auto_summarize)
    };
    settings.validate_models().map_err(|e| e.to_string())?;
    // The theme is not this command's to write. `set_theme` owns it, and the
    // drawer's draft carries whatever the theme was when it opened — so a Save
    // of some unrelated field would put that stale value back and silently undo
    // a theme picked in between. One writer, and the incoming value is ignored
    // rather than validated.
    settings.theme = state.settings.lock().theme.clone();
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
    // A model nobody is going to ask for again should not stay resident. This
    // fires when the user picks a different local model — usually a smaller one,
    // and usually because the larger did not comfortably fit — or leaves local
    // models behind for a cloud provider. Both used to keep the old weights in
    // memory until something happened to reload, which on the machine that most
    // needed the room was exactly the wrong answer.
    // Turning auto-summarize off is the second trigger, and it is the one the
    // settings screen makes a promise about: it says no language model is
    // loaded, and a model already resident from an earlier summary would make
    // that a lie. Asking for a summary by hand afterwards loads it again, which
    // is the point — it happens when the user asks.
    let stopped_summarising = previously_automatic && !settings.auto_summarize;
    if stopped_summarising || wanted_local_weights(&settings) != previously_wanted {
        crate::llm::local::release_model();
    }
    Ok(settings)
}

#[tauri::command]
pub fn save_settings(
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    Ok(persist_settings(&state, settings)?.public_view())
}

/// Write the theme and nothing else.
///
/// The header toggle used to send a whole settings snapshot for one field, which
/// made it a lost-update waiting to happen: open the drawer, save something, and
/// a toggle still in flight lands afterwards carrying the pre-drawer value of
/// every other field. Reading the current row here and changing one member of it
/// under the lock means the two writers cannot disagree about anything they did
/// not each touch.
#[tauri::command]
pub fn set_theme(state: State<'_, Arc<AppState>>, theme: String) -> Result<AppSettings, String> {
    // The WebView is the trust boundary. `persist_settings` rejects anything
    // outside the pair and this path must too, or it becomes the way around it.
    if theme != "light" && theme != "dark" {
        return Err("invalid theme: expected `light` or `dark`".into());
    }
    // Whichever home is holding the key keeps holding it. Hardcoding `Keychain`
    // here would strip the key from the row on a machine whose keychain refused
    // to cooperate — the one place it is still stored — and a theme toggle would
    // silently cost the user their credential.
    let home = if state.key_in_keychain.load(Ordering::Relaxed) {
        KeyHome::Keychain
    } else {
        KeyHome::KeepInRow
    };
    // The guard is held across the database write. Releasing it first left a
    // window in which a concurrent save could land between the read and the
    // write, and the loser's whole snapshot would overwrite the winner's row.
    let mut current = state.settings.lock();
    current.theme = theme;
    state.db.save_settings_with(&current, home)?;
    Ok(current.public_view())
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

/// Fold a fresh set of suggestions into the meeting's action items.
///
/// The model may replace its own untouched suggestions and nothing else — the
/// rule lives in `domain::actions::merge_suggestions` and is tested there. This
/// is the plumbing: read what is stored, merge, write both the rows and the
/// text column that export and search read.
///
/// Errors are swallowed on purpose. A summary that arrived is worth keeping
/// even if the task list could not be updated, and the alternative — failing
/// the whole summarise — would throw away the expensive half over the cheap one.
fn apply_action_items(state: &AppState, id: &str, insights: &MeetingInsights) {
    let mut existing = state.db.list_action_items(id).unwrap_or_default();
    if existing.is_empty() {
        // A meeting summarised before this existed has its items only in the
        // text column. Read them back rather than letting the first merge write
        // over a list somebody may have been relying on.
        //
        // Marked as touched, because there is no way to know which of them a
        // person had already corrected — the old column kept no such record.
        // The cost is that a stale suggestion survives until it is deleted by
        // hand; the alternative is deleting work nobody agreed to lose.
        existing = state
            .db
            .get_meeting(id)
            .ok()
            .flatten()
            .and_then(|m| m.action_items)
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .map(|l| l.trim_start_matches('-').trim())
                    .filter(|l| !l.is_empty())
                    .map(|l| crate::domain::actions::ActionItem {
                        id: 0,
                        text: l.to_string(),
                        owner: None,
                        due: None,
                        status: crate::domain::actions::ActionStatus::Open,
                        source: crate::domain::actions::ActionSource::Ai,
                        edited: true,
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let merged = crate::domain::actions::merge_suggestions(&existing, &insights.action_items);
    if let Err(e) = state.db.save_action_items(id, &merged) {
        tracing::warn!("action items could not be updated: {e}");
    }
}

#[tauri::command]
pub fn list_action_items(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Vec<crate::domain::actions::ActionItem>, String> {
    state.db.list_action_items(&id)
}

/// One item at a time, never the whole list.
///
/// A client that sends the list sends its idea of every *other* item with it,
/// and that idea is stale the moment anything else writes — a summary merging
/// in the background, or the same panel a second earlier. Every way this could
/// lose somebody's work went through exactly that, so the list is not something
/// the client is allowed to state.
fn validated(mut item: ActionItem) -> Result<ActionItem, String> {
    item.text = item.text.trim().to_string();
    if item.text.is_empty() {
        return Err("a task needs something in it".into());
    }
    if item.text.chars().count() > 2_000 {
        return Err("that task is too long".into());
    }
    let tidy = |v: &Option<String>| {
        v.as_ref().and_then(|x| {
            let x = x.trim();
            (!x.is_empty()).then(|| x.to_string())
        })
    };
    item.owner = tidy(&item.owner);
    item.due = tidy(&item.due);
    Ok(item)
}

#[tauri::command]
pub async fn add_action_item(
    state: State<'_, Arc<AppState>>,
    id: String,
    text: String,
) -> Result<Vec<ActionItem>, String> {
    let _flight = state.refine_flight.lock().await;
    if state.db.get_meeting(&id)?.is_none() {
        return Err("meeting not found".into());
    }
    let item = validated(ActionItem {
        id: 0,
        text,
        owner: None,
        due: None,
        status: crate::domain::actions::ActionStatus::Open,
        // A person wrote it, so no summary may take it away.
        source: crate::domain::actions::ActionSource::User,
        edited: true,
    })?;
    state.db.insert_action_item(&id, &item)?;
    state.db.list_action_items(&id)
}

/// Change one item. The source is not taken from the caller — an existing row
/// keeps whose it was, so nothing can promote the model's suggestion into
/// something a person is supposed to have said.
#[tauri::command]
pub async fn update_action_item(
    state: State<'_, Arc<AppState>>,
    id: String,
    item: ActionItem,
) -> Result<Vec<ActionItem>, String> {
    let _flight = state.refine_flight.lock().await;
    let item = validated(item)?;
    state.db.update_action_item(&id, &item)?;
    state.db.list_action_items(&id)
}

#[tauri::command]
pub async fn delete_action_item(
    state: State<'_, Arc<AppState>>,
    id: String,
    item_id: i64,
) -> Result<Vec<ActionItem>, String> {
    let _flight = state.refine_flight.lock().await;
    state.db.delete_action_item(&id, item_id)?;
    state.db.list_action_items(&id)
}

/// Correct one line of a transcript.
///
/// Local speech-to-text mishears names, acronyms and one-word answers, and a
/// person fixing the line is the fastest route to notes worth trusting. The
/// timing is not editable: it came from the audio, and the summary, the search
/// index and any future alignment all read it.
///
/// Refused while that meeting is recording. The live transcript is being
/// appended to by the transcription ticker, which writes the whole set back —
/// an edit made in that window would be silently overwritten by the next chunk,
/// which is worse than not offering it.
#[tauri::command]
pub async fn edit_transcript_segment(
    state: State<'_, Arc<AppState>>,
    id: String,
    segment_id: String,
    text: String,
) -> Result<LiveTranscript, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a line cannot be emptied — delete is a different action".into());
    }
    // Bounded for the same reason a note is: this reaches the summary prompt.
    if text.chars().count() > 4_000 {
        return Err("that line is too long".into());
    }
    let recording_this = state
        .active_meeting
        .lock()
        .as_deref()
        .is_some_and(|active| active == id)
        && state.recorder.is_recording();
    if recording_this {
        return Err("the transcript can be corrected once the recording has stopped".into());
    }
    // The same lock the final pass and a retranscription take. Without it the
    // recorder stopping is not enough: `stop_recording` is still awaiting the
    // last chunk with a transcript it cloned before this edit existed, and it
    // writes that clone back — so a correction made in the window between the
    // capture closing and that write would vanish with no sign it had been
    // made. Held across the whole read-modify-write below, because the value
    // being protected is the transcript, not any one statement about it.
    let _flight = state.stt_flight.lock().await;

    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let mut transcript = match state.live.lock().get(&id) {
        Some(t) => t.clone(),
        None => state.db.load_transcript(&id)?,
    };
    if !transcript.edit_segment(&segment_id, text) {
        return Err("that line is no longer in this transcript".into());
    }

    // The summary reads `transcript_text`, not the segments, so the correction
    // has to land there or it would be visible on screen and invisible to the
    // model. All three writes — segments, that field, and the search index —
    // commit together or not at all.
    meeting.transcript_text = transcript.plain_text();
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.save_corrected_transcript(&meeting, &transcript)?;
    // The cache is what `get_transcript` answers from while it is warm.
    state.live.lock().insert(id.clone(), transcript.clone());
    Ok(transcript)
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

/// Async and off the runtime, because probing costs seconds.
///
/// A non-async `#[tauri::command]` runs on the main thread, and this one asks
/// `nvidia-smi` for a name, opens ggml's backend libraries and enumerates every
/// Vulkan device on the machine. On a laptop with two GPUs that is long enough
/// to freeze the window while Settings is opening — which is exactly when it is
/// called.
#[tauri::command]
pub async fn get_capabilities() -> Result<CapabilityReport, String> {
    tokio::task::spawn_blocking(detect_capabilities)
        .await
        .map_err(|e| format!("capability probe failed: {e}"))
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
    app: AppHandle,
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
    let given = title.filter(|t| !t.trim().is_empty());
    // A name the caller supplied is the user's from the start; only the date
    // label is ours to replace once a summary exists.
    let title_locked = given.is_some();
    let title = given.unwrap_or_else(|| fallback_title(chrono::Local::now()));
    let audio_path = recordings_dir().join(format!("{id}.wav"));
    // Claimed before the capture opens, not after. Between those two points the
    // recorder answers "yes, recording" while this still named the previous
    // meeting, and a context note landing in that gap would be stamped with this
    // recording's clock and filed against the last one.
    //
    // The claim is also what makes two overlapping starts safe. The check at the
    // top of this command reads a flag the recorder only sets once capture is
    // open, so two calls can both pass it; only one can find this empty.
    {
        let mut active = state.active_meeting.lock();
        if active.is_some() {
            return Err("already recording".into());
        }
        *active = Some(id.clone());
    }
    if let Err(e) = state.recorder.start(
        audio_path.clone(),
        settings.mic_device_id.clone(),
        settings.system_device_id.clone(),
    ) {
        // Give the claim back, and only if it is still ours — the same
        // compare-before-clear the stop path uses. A meeting that never started
        // recording must not be left owning the recorder, and a meeting that
        // did must not have its claim taken away by someone else's failure.
        let mut active = state.active_meeting.lock();
        if active.as_deref() == Some(id.as_str()) {
            *active = None;
        }
        return Err(e.to_string());
    }
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
        title_locked,
        cost_nano_usd: None,
        cost_label: None,
    };
    state.db.upsert_meeting(&meeting)?;
    state.live.lock().insert(id.clone(), LiveTranscript::new());
    // Started here rather than by the window, so transcription keeps running when
    // the window is minimized and its timers are throttled to a crawl.
    spawn_live_stt_ticker(app.clone(), Arc::clone(&state));
    // A recording can be started by the tray or the accelerator while the window
    // is already minimized, so the card has to be considered here and not only
    // when the window is minimized.
    sync_overlay(&app, &state, false);
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
    // Retire the ticker with the recording it belongs to. `is_recording` is
    // already false above, so it would stop on its own within 1200ms — bumping
    // the generation makes it immediate, and makes a start that follows quickly
    // unambiguous about which ticker owns the microphone.
    state.live_stt_generation.fetch_add(1, Ordering::SeqCst);
    // Before the tail transcription and the summary, which take seconds: the
    // card must not sit there advertising a recording that has already stopped.
    sync_overlay(&app, &state, false);
    let duration = state.recorder.elapsed_ms();
    // Take the tail here, synchronously, while this is still the only thing that
    // has touched the recorder since it stopped. Draining it later — after the
    // await for `stt_flight` — read whatever buffer existed by then, and a
    // recording started in the meantime owns that buffer: its opening seconds
    // were transcribed into the meeting that had just ended, and left as a hole
    // in the one that had just begun.
    let tail = state.recorder.drain_chunks();
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
    // Compare before clearing. This command awaits `stt_flight` for the final
    // pass, and a recording started in the meantime has already written its own
    // id here — blanking it would leave that recorder running with pause, resume
    // and stop all answering "no active meeting", and no way to finish it.
    {
        let mut active = state.active_meeting.lock();
        if active.as_deref() == Some(id.as_str()) {
            *active = None;
        }
    }
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

    // Whether a pass *ran*, not whether it produced words. A paid pass can answer
    // with no text and still have been billed, and reading the empty transcript
    // as "live transcription never happened" sent the whole recording through
    // the provider again — charging twice for the same audio.
    if !state.live_stt_passes.lock().remove(&id) {
        // Nothing was transcribed live — cloud STT down, or a recording short
        // enough that no poll ever ran. Transcribe the whole saved WAV.
        if let Ok((mic, sys, sr)) = read_dual_wav(&path) {
            if let Ok(chunks) = state
                .stt
                .transcribe_dual(&settings, &mic, &sys, sr, 0)
                .await
            {
                bill_chunks(&state.db, &id, &chunks);
                apply_stt_chunks(&mut live, &chunks);
            }
        }
    } else {
        // Live transcription only ever consumed what the last poll drained, so
        // everything spoken between that drain and the stop is still sitting in
        // the buffer. Skipping it — which is what happened whenever any live
        // segment existed — silently dropped the end of every meeting.
        let (mic, sys, sr) = tail;
        if !mic.is_empty() || !sys.is_empty() {
            let tail_ms = duration
                .saturating_sub((mic.len().max(sys.len()) as u64 * 1000) / sr.max(1) as u64);
            match state
                .stt
                .transcribe_dual(&settings, &mic, &sys, sr, tail_ms)
                .await
            {
                Ok(chunks) => {
                    bill_chunks(&state.db, &id, &chunks);
                    apply_stt_chunks(&mut live, &chunks)
                }
                // Same reason as the summary above: the error names the model.
                Err(_) => tracing::warn!("final chunk could not be transcribed"),
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
        // The same flight every other whole-set replacement takes. A refinement
        // started before Stop can be awaiting its model call right now, and
        // without this its result would be overwritten by the snapshot this path
        // has been holding since before the call was made.
        let _flight = state.refine_flight.lock().await;
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
                &state.db.list_context_notes(&id).unwrap_or_default(),
            )
            .await
        {
            Ok((insights, cost)) => {
                state.db.save_insights(&id, &insights)?;
                // A meeting's first insights are version 1. Recording it here
                // rather than lazily means the history is complete from the
                // start instead of from whenever someone first opened it.
                state.db.push_summary_version(&id, "summarize", &insights)?;
                state.db.add_meeting_cost(&id, cost)?;
                meeting.summary = Some(insights.summary.clone());
                meeting.action_items = Some(insights.action_items_text());
                meeting.key_points = Some(insights.key_points_text());
                meeting.status = MeetingStatus::Ready;
                name_meeting(&state, &settings, &mut meeting, &insights.summary).await;
                meeting.updated_at = chrono::Utc::now().to_rfc3339();
                state.db.upsert_meeting(&meeting)?;
                // Last, because the upsert above writes the model's list into
                // `action_items` and this writes the merged one over it. The
                // other order left every protected item out of exports and
                // search while the rows still held them.
                apply_action_items(&state, &id, &insights);
            }
            // The transcript is already saved and the meeting is already Ready,
            // so propagating this told the user their recording was lost when
            // only the summary was. Report the summary, keep the meeting.
            Err(e) => {
                // The provider's error is not repeated. It carries the model id,
                // which arrives from the WebView, and this lands in a file on the
                // user's disk. `done` below still carries the detail to the
                // window, which is where the person who can act on it is looking.
                tracing::warn!("auto-summary failed");
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

/// Cadence of the backend live-STT ticker.
const LIVE_STT_INTERVAL_MS: u64 = 1200;

/// Drain whatever audio has arrived and transcribe it.
///
/// No longer a command. It used to be driven by a `setInterval` in the window,
/// and WebView2 treats a minimized window as a hidden page: its timers clamp to
/// roughly one a second and then to one a minute after five. Since this call is
/// what *drains* the recorder, that did not merely slow the display down — it
/// stalled transcription itself for anyone who minimized the app during a
/// meeting, which is precisely when they would.
async fn drive_live_stt(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let id = state
        .active_meeting
        .lock()
        .clone()
        .ok_or_else(|| "no active meeting".to_string())?;
    if !state.recorder.is_recording() || state.recorder.is_paused() {
        return Ok(());
    }
    // Single-flight. The UI polls every 1200ms and a chunk can take longer than
    // that to transcribe, so without this the next poll drains a second slice of
    // audio while the first is still running. Both then timestamp their slice from
    // whatever `elapsed_ms` reads at drain time, and the transcript comes out in
    // the wrong order. Overlapping polls now just return what is already there;
    // the audio stays in the buffer for the next turn.
    let Ok(_flight) = state.stt_flight.try_lock() else {
        return Ok(());
    };
    let (mic, sys, sr) = state.recorder.drain_chunks();
    if mic.is_empty() && sys.is_empty() {
        return Ok(());
    }
    let start_ms = state
        .recorder
        .elapsed_ms()
        .saturating_sub((mic.len().max(sys.len()) as u64 * 1000) / sr.max(1) as u64);
    let settings = state.settings.lock().clone();
    let chunks = match state
        .stt
        .transcribe_dual(&settings, &mic, &sys, sr, start_ms)
        .await
    {
        Ok(chunks) => chunks,
        Err(e) => {
            // The audio was drained before the call. Propagating without putting
            // it back threw a slice of the meeting away every 1200ms — so a cloud
            // provider rejecting every chunk silently shredded the live
            // transcript while the window showed nothing at all. The cursor goes
            // back by what was taken and the next poll tries the same audio again.
            state.recorder.rewind_chunks(mic.len(), sys.len());
            return Err(e);
        }
    };
    // Charged before the transcript is merged. The sum goes through SQL rather
    // than a read-modify-write here: two channels transcribe concurrently and
    // one would overwrite the other.
    bill_chunks(&state.db, &id, &chunks);
    state.live_stt_passes.lock().insert(id.clone());
    let mut guard = state.live.lock();
    let t = guard.entry(id.clone()).or_default();
    apply_stt_chunks(t, &chunks);
    let snapshot = t.clone();
    drop(guard);
    let _ = app.emit("transcript://append", &snapshot);
    Ok(())
}

/// Drive live transcription from the backend for as long as this recording lasts.
///
/// Retired by generation rather than by a stop flag: a stop followed quickly by a
/// start would otherwise leave two tickers alive, splitting each window of audio
/// between them and running concurrent transcriptions over halves of the same
/// speech. The check runs before the work, never mid-flight, so a pass already
/// running still lands.
fn spawn_live_stt_ticker(app: AppHandle, state: Arc<AppState>) {
    let generation = state.live_stt_generation.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(LIVE_STT_INTERVAL_MS)).await;
            if state.live_stt_generation.load(Ordering::SeqCst) != generation
                || !state.recorder.is_recording()
            {
                break;
            }
            if let Err(e) = drive_live_stt(&app, &state).await {
                // The window has to be told, or a provider rejecting every chunk
                // is a screen that simply never fills. The audio is still being
                // captured and saved, so this is degraded rather than lost — and
                // the message carries no model id, which arrives from the WebView
                // and must not reach the log this may also be written to.
                let _ = app.emit("transcript://error", &e);
            }
        }
    });
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
    // Provenance first, shape second. The flag is the answer for anything named
    // since it existed; the shape test still covers meetings recorded before the
    // column, whose flag defaults to false and whose title is genuinely ours.
    if meeting.title_locked || !is_fallback_title(&meeting.title) {
        return;
    }
    match state
        .llm
        .title(settings, summary, &meeting.transcript_text)
        .await
    {
        Ok((raw, cost)) => {
            // Charged to the meeting even when the answer is unusable: the call
            // was made and the provider billed it.
            if let Err(e) = state.db.add_meeting_cost(&meeting.id, cost) {
                tracing::warn!("could not record the cost of a generated title: {e}");
            }
            match parse_title(&raw) {
                Some(title) => meeting.title = title,
                None => tracing::warn!("the model answered with no usable title"),
            }
        }
        // The provider's error is not repeated. It carries the model id, which
        // arrives from the WebView, and this now lands in a file on the user's
        // disk. A failed title is a nuisance, not something worth widening what
        // the log holds.
        Err(_) => tracing::warn!("title generation failed"),
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
    // Held for the same reason a refinement holds it: this replaces the whole
    // insight set, and a refinement awaiting its model call would otherwise
    // write a version built from what this is about to overwrite.
    let _flight = state.refine_flight.lock().await;
    let (insights, cost) = state
        .llm
        .summarize(
            &settings,
            &m.transcript_text,
            tpl,
            // Empty on failure rather than refusing to summarise: a note that
            // cannot be read is a worse summary, not a lost meeting.
            &state.db.list_context_notes(&id).unwrap_or_default(),
        )
        .await?;
    // The summary being replaced has to become a version before it is gone.
    // Without this, re-summarising a meeting nobody had opened the history of
    // left the original unrecoverable — the lazy baseline would then record the
    // replacement as if it had always been the first.
    if m.summary.is_some() && state.db.list_summary_versions(&id)?.is_empty() {
        state
            .db
            .push_summary_version(&id, "summarize", &current_insights(&m))?;
    }
    state.db.save_insights(&id, &insights)?;
    state.db.push_summary_version(&id, "summarize", &insights)?;
    state.db.add_meeting_cost(&id, cost)?;
    m.status = MeetingStatus::Ready;
    m.summary = Some(insights.summary.clone());
    m.action_items = Some(insights.action_items_text());
    m.key_points = Some(insights.key_points_text());
    // Re-summarising is also the way an old meeting still carrying its date
    // label gets a real name.
    name_meeting(&state, &settings, &mut m, &insights.summary).await;
    m.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&m)?;
    // After the upsert, which wrote the model's list; this writes the merged
    // one over it.
    apply_action_items(&state, &id, &insights);
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
    // The same corrections the summary gets. A question about a client's name
    // should be answered from what the participant typed, not from what the
    // transcriber heard.
    let notes = state.db.list_context_notes(&id).unwrap_or_default();
    let settings = state.settings.lock().clone();
    state.db.add_chat(&id, "user", &question)?;
    let (answer, cost) = state
        .llm
        .chat(
            &settings,
            crate::domain::context::ChatSubject {
                meeting_title: &meeting.title,
                transcript: &meeting.transcript_text,
                summary: meeting.summary.as_deref(),
                notes: &notes,
            },
            &history,
            &question,
        )
        .await?;
    // Billed to the meeting it is about: it is the same OpenRouter spend and the
    // user is looking at that meeting's total.
    state.db.add_meeting_cost(&id, cost)?;
    state.db.add_chat(&id, "assistant", &answer)?;
    Ok(ChatMessage {
        role: "assistant".into(),
        content: answer,
    })
}

/// A note the participant typed while the meeting was happening.
///
/// `at_ms` comes from the recorder rather than from the caller: the WebView
/// knows what it painted, not where the recording actually is, and a note that
/// claims a position the audio never had is worse than one with no position.
#[tauri::command]
pub fn add_context_note(
    state: State<'_, Arc<AppState>>,
    id: String,
    text: String,
) -> Result<crate::domain::context::ContextNote, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a note needs something in it".into());
    }
    // Bounded because it reaches a prompt. Not a security boundary — the person
    // typing owns the machine — but a megabyte pasted here would push the
    // transcript out of the model's window and quietly ruin the summary.
    if text.chars().count() > 2_000 {
        return Err("that note is too long".into());
    }
    // The stamp is the thing that has to be earned, not the note. A note about
    // the meeting being recorded gets the recorder's position; a note about any
    // other meeting gets none.
    //
    // That distinction is the security boundary. The id arrives from the
    // WebView while the stamp comes from the recorder, so handing this
    // recording's timestamp to an arbitrary meeting would let a caller write
    // evidence of a position the audio never had — and notes are told to win
    // over the transcript. A note with no position claims nothing.
    let recording_this = state
        .active_meeting
        .lock()
        .as_deref()
        .is_some_and(|active| active == id)
        && state.recorder.is_recording();
    let at_ms = recording_this.then(|| state.recorder.elapsed_ms() as i64);
    // The meeting still has to exist. Without this the foreign key would be the
    // only thing refusing, and it would do it with a SQLite error rather than a
    // sentence.
    if state.db.get_meeting(&id)?.is_none() {
        return Err("meeting not found".into());
    }
    state.db.add_context_note(&id, text, at_ms)
}

#[tauri::command]
pub fn list_context_notes(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Vec<crate::domain::context::ContextNote>, String> {
    state.db.list_context_notes(&id)
}

#[tauri::command]
pub fn delete_context_note(
    state: State<'_, Arc<AppState>>,
    id: String,
    note_id: i64,
) -> Result<(), String> {
    state.db.delete_context_note(&id, note_id)
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
    let given = title
        .filter(|t| !t.trim().is_empty())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()));
    // A supplied name and a file stem are both real names — an imported
    // `weekly-sync.wav` is called that on purpose, and the generator has no
    // business renaming it. Only the last resort is ours.
    let title_locked = given.is_some();
    let title = given.unwrap_or_else(|| "Imported meeting".into());
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
        title_locked,
        cost_nano_usd: None,
        cost_label: None,
    };
    state.db.upsert_meeting(&meeting)?;
    let settings = state.settings.lock().clone();
    // Mono import: Me channel only (no dual split in source file)
    let chunks = state
        .stt
        .transcribe_dual(&settings, &pcm, &[], sr, 0)
        .await?;
    let mut t = LiveTranscript::new();
    // Billed like every other transcription path: this one calls the same
    // provider and it was the meeting's only charge on an imported file.
    bill_chunks(&state.db, &id, &chunks);
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
    // Billed like every other transcription path: this one calls the same
    // provider and it was the meeting's only charge on an imported file.
    bill_chunks(&state.db, &id, &chunks);
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
    // A model download is egress like any other — the catalogue lives on the
    // internet, and the switch says nothing leaves.
    if let Some(refusal) = crate::domain::offline::refuse(
        state.settings.lock().offline_mode,
        crate::domain::offline::Egress::Download,
    ) {
        return Err(refusal);
    }
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
/// Charge a meeting for a batch of transcription chunks.
///
/// `Some(0)` and `None` are different answers and the difference is the whole
/// point of the column: a free cloud model reported a real zero and the meeting
/// should read `$0.00`, while a meeting transcribed on this machine reported
/// nothing and should show no price at all. Summing to zero and skipping the
/// write would have collapsed the first into the second.
fn bill_chunks(db: &Database, id: &str, chunks: &[SttChunkResult]) -> Option<i64> {
    let mut total: Option<i64> = None;
    for c in chunks {
        if let Some(n) = c.cost_nano_usd {
            total = Some(total.unwrap_or(0) + n);
        }
    }
    if let Some(total) = total {
        if let Err(e) = db.add_meeting_cost(id, Some(total)) {
            // A bookkeeping row is not worth losing a transcript over.
            tracing::warn!("could not record a transcription charge: {e}");
        }
    }
    total
}

/// Show, hide, move and size the minimized-recording card.
///
/// Both inputs are re-read here rather than remembered: a recording can stop
/// while the window is minimized and the window can be restored while recording,
/// and a flag toggled by whichever event fired last gets one of those wrong.
fn sync_overlay(app: &AppHandle, state: &AppState, expanded: bool) {
    let Some(overlay) = app.get_webview_window("overlay") else {
        return;
    };
    let recording = state.recorder.is_recording();
    let minimized = app
        .get_webview_window("main")
        .and_then(|w| w.is_minimized().ok())
        .unwrap_or(false);

    if !overlay_visible(recording, minimized) {
        let _ = overlay.hide();
        return;
    }

    let size = if expanded { EXPANDED } else { COLLAPSED };
    // The monitor the main window is on, not the primary: on a two-screen desk
    // the card belongs beside the work, and the scale factor differs per display.
    let monitor = app
        .get_webview_window("main")
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| overlay.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let pos = m.position();
        let msize = m.size();
        let (x, y) = dock_right_center(
            (pos.x, pos.y),
            (msize.width, msize.height),
            m.scale_factor(),
            size,
        );
        let _ = overlay.set_size(tauri::LogicalSize::new(size.0, size.1));
        let _ = overlay.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = overlay.show();
}

/// Re-derive the card's visibility after the main window moved or changed state.
///
/// Collapsed on purpose: the pointer is not over the card at the moment the
/// window is minimized, and starting expanded would put a 340px panel on screen
/// that nothing asked for.
pub fn sync_overlay_for(app: &AppHandle, state: &AppState) {
    sync_overlay(app, state, false);
}

/// The card asking to grow or shrink as the pointer arrives and leaves.
#[tauri::command]
pub fn set_overlay_expanded(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    expanded: bool,
) -> Result<(), String> {
    sync_overlay(&app, &state, expanded);
    Ok(())
}

/// Rebuild the insight set a meeting currently shows, so a refinement can be
/// stored as a version without reparsing its markdown twice.
fn current_insights(m: &MeetingRecord) -> MeetingInsights {
    MeetingInsights {
        summary: m.summary.clone().unwrap_or_default(),
        key_points: bullets(m.key_points.as_deref()),
        action_items: bullets(m.action_items.as_deref()),
    }
}

fn bullets(text: Option<&str>) -> Vec<String> {
    text.map(parse_refined_list).unwrap_or_default()
}

/// Every stored version of a meeting's insights, oldest first.
///
/// Backfills a baseline on first read for meetings summarised before versioning
/// existed: without it their first improvement would be version 1 and the
/// original would be the thing that vanished. A write on read, and idempotent —
/// the next call finds the baseline already there.
#[tauri::command]
pub fn list_summary_versions(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Vec<SummaryVersion>, String> {
    let existing = state.db.list_summary_versions(&id)?;
    if !existing.is_empty() {
        return Ok(existing);
    }
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    if meeting.summary.is_none() {
        return Ok(existing);
    }
    state
        .db
        .push_summary_version(&id, "summarize", &current_insights(&meeting))?;
    state.db.list_summary_versions(&id)
}

/// Ask the model to improve one section, keeping everything it replaces.
#[tauri::command]
pub async fn refine_summary_section(
    state: State<'_, Arc<AppState>>,
    id: String,
    section: String,
) -> Result<SummaryVersion, String> {
    // From the WebView, so it is parsed rather than trusted: an unrecognised
    // value picks no prompt and merges into nothing, and failing here says so.
    let section = Section::parse(&section).ok_or_else(|| "unknown section".to_string())?;
    // Taken before the read and held past the write, so every version is built
    // on what the one before it produced.
    let _flight = state.refine_flight.lock().await;
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    if meeting.summary.is_none() {
        return Err("summarize this meeting before improving it".into());
    }
    // The baseline has to exist before the improvement is written, or the
    // original is what gets lost.
    if state.db.list_summary_versions(&id)?.is_empty() {
        state
            .db
            .push_summary_version(&id, "summarize", &current_insights(&meeting))?;
    }

    let settings = state.settings.lock().clone();
    let current = match section {
        Section::KeyPoints => meeting.key_points.clone().unwrap_or_default(),
        Section::ActionItems => meeting.action_items.clone().unwrap_or_default(),
    };
    // The timestamped, speaker-labelled transcript rather than the flattened
    // text the first pass used: the whole transcript already went in, so "more
    // information" is the structure, not more of it.
    let transcript = state
        .live
        .lock()
        .get(&id)
        .map(|t| t.timestamped_text())
        .unwrap_or_else(|| meeting.transcript_text.clone());
    let prompt = build_refine_prompt(section, &current, &transcript, settings.locale());
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let (raw, cost) = state.llm.complete_for_refine(&settings, &messages).await?;
    state.db.add_meeting_cost(&id, cost)?;

    let improved = parse_refined_list(&raw);
    if improved.is_empty() {
        // A model that answered with prose has not produced a list. Writing it
        // would replace good notes with a sentence about not improving them.
        return Err("the model did not answer with a list".into());
    }
    let mut insights = current_insights(&meeting);
    match section {
        Section::KeyPoints => insights.key_points = improved,
        Section::ActionItems => insights.action_items = improved,
    }
    let version = state
        .db
        .push_summary_version(&id, section.as_str(), &insights)?;
    state.db.save_insights(&id, &insights)?;
    meeting.summary = Some(insights.summary.clone());
    meeting.key_points = Some(insights.key_points_text());
    meeting.action_items = Some(insights.action_items_text());
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    apply_action_items(&state, &id, &insights);
    Ok(version)
}

/// Put an earlier version back, as a new version.
///
/// Appended rather than rewound: history stays append-only, so restoring is
/// itself undoable and nothing the user has seen ever disappears.
#[tauri::command]
pub async fn restore_summary_version(
    state: State<'_, Arc<AppState>>,
    id: String,
    version: i64,
) -> Result<SummaryVersion, String> {
    // The same flight a refinement holds. A restore landing while one is
    // awaiting its model call would be overwritten the moment that call returned
    // with the pre-restore snapshot it had been holding all along.
    let _flight = state.refine_flight.lock().await;
    let wanted = state
        .db
        .list_summary_versions(&id)?
        .into_iter()
        .find(|v| v.version == version)
        .ok_or_else(|| "version not found".to_string())?;
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let insights = MeetingInsights {
        summary: wanted.summary.clone(),
        key_points: parse_refined_list(&wanted.key_points),
        action_items: parse_refined_list(&wanted.action_items),
    };
    let created = state.db.push_summary_version(&id, "restore", &insights)?;
    state.db.save_insights(&id, &insights)?;
    meeting.summary = Some(insights.summary.clone());
    meeting.key_points = Some(insights.key_points_text());
    meeting.action_items = Some(insights.action_items_text());
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    // A restore is the user asking for an older set, and it still goes through
    // the merge: what they have edited or ticked since is theirs, and a restore
    // of the prose is not a request to undo their task list.
    apply_action_items(&state, &id, &insights);
    Ok(created)
}

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
    // From here the name is the user's. Nothing generated replaces it, however
    // much the shape of what they typed happens to resemble the date label.
    meeting.title_locked = true;
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
