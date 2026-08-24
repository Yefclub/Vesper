use crate::audio::capture::{read_dual_wav, repair_wav_header, DualChannelRecorder};
use crate::audio::decode::decode_audio_file;
use crate::audio::devices::{list_audio_devices, AudioDevice};
use crate::db::{Database, KeyHome};
use crate::domain::actions::ActionItem;
use crate::domain::capabilities::{detect_capabilities, CapabilityReport};
use crate::domain::channels::ChannelSelection;
use crate::domain::chat::ChatMessage;
use crate::domain::export::{build_markdown, export_meeting, safe_file_stem, ExportFormat};
use crate::domain::gate::{can_start_recording_with, StartGate};
use crate::domain::i18n::{catalog, t, Locale};
use crate::domain::job::{
    MeetingEvent, MeetingPhase, MeetingProgress, MeetingRecord, MeetingStatus,
};
use crate::domain::overlay::{dock_at, footprints, overlay_visible, OverlayPosition};
use crate::domain::playback::{playable_recording, Unplayable};
use crate::domain::recovery::{interrupted, is_interrupted};
use crate::domain::refine::{build_refine_prompt, parse_refined_list, Section, SummaryVersion};
use crate::domain::search::SearchHit;
use crate::domain::segmenter::{Segmenter, Utterance};
use crate::domain::settings::{AppSettings, LlmProvider, SttProvider};
use crate::domain::shortcut::ShortcutStatus;
use crate::domain::speaker::{clean_speaker_name, Speaker, SpeakerNames};
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
    /// Where each channel's live audio is cut into utterances.
    ///
    /// One per channel, not one for both: two people pause in different places,
    /// and a boundary found in the microphone means nothing in what the speakers
    /// are playing. `.0` is the microphone, `.1` the system.
    ///
    /// Keyed by meeting, like `live`, and for the same reason. A single global
    /// pair is a thing two recordings can both reach: a stop racing a live pass
    /// that is still transcribing would take the buffer out from under it, and
    /// the utterance that pass then failed to transcribe would be put back into
    /// whatever pair had replaced it — the next meeting's, or nobody's.
    pub segmenters: Mutex<HashMap<String, (Segmenter, Segmenter)>>,
    /// A native dialog this application opened is on screen.
    ///
    /// Set by the window around the file pickers it opens, because from the
    /// outside a modal chooser and another application look the same: the main
    /// window loses focus either way, and only one of them is a reason to float
    /// a card over the screen.
    pub modal_open: AtomicBool,
    /// Where the recording stands with respect to the quiet, and when anybody
    /// last said anything.
    ///
    /// Both live here rather than in the segmenter: it drops silence without
    /// counting it, which is right for transcription and useless for noticing
    /// that a room has been empty for three minutes.
    /// Picks moved off a model the catalogue no longer offers, waiting to be
    /// said out loud once.
    ///
    /// The migration runs in `new`, before there is a window to tell — and a
    /// summary written by a different model than yesterday's is not something
    /// to leave in a log file. Drained by the command that reads it, so the
    /// notice appears once rather than at every launch.
    pub retired_models: Mutex<Vec<(String, String)>>,
    /// Said once per recording. A banner that reappears every 1200 ms is not a
    /// warning, it is a fault of its own.
    pub warned_deaf: Mutex<crate::domain::deaf::Deaf>,
    pub vigil: Mutex<crate::domain::silence::Vigil>,
    pub last_speech_ms: AtomicU64,
    pub stt: SttService,
    pub llm: LlmService,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        ensure_app_dirs()?;
        let data = crate::paths::app_data_dir();
        let db = Database::open(&data)?;
        let mut settings = db.load_settings().unwrap_or_default();
        // Before anything reads the pick. A row still naming a model the
        // catalogue dropped cannot be verified and cannot be recorded with, and
        // the picker does not list it — so the user would find the application
        // refusing to record over a model they can neither fix nor see.
        //
        // Written back, not only held: the next save would otherwise carry the
        // dead id again from whatever the drawer had loaded.
        let retired = settings.migrate_retired_models();
        let (key, in_keychain) = load_or_migrate_api_key(&db);
        settings.openrouter_api_key = key;
        // After the key is resolved, and told where the key lives. Writing the
        // row before this point would have written it with whatever
        // `load_settings` happened to return and a `Keychain` home — which is
        // how the row drops its copy, and at that moment the row is still the
        // only copy on a machine whose keychain has not answered yet.
        if !retired.is_empty() {
            for (from, to) in &retired {
                tracing::info!("model `{from}` is no longer offered — moved to `{to}`");
            }
            let home = if in_keychain {
                KeyHome::Keychain
            } else {
                KeyHome::KeepInRow
            };
            let _ = db.save_settings_with(&settings, home);
        }
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
            segmenters: Mutex::new(HashMap::new()),
            modal_open: AtomicBool::new(false),
            retired_models: Mutex::new(retired),
            warned_deaf: Mutex::new(crate::domain::deaf::Deaf::default()),
            vigil: Mutex::new(crate::domain::silence::Vigil::default()),
            last_speech_ms: AtomicU64::new(0),
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
    /// The channels the running capture is listening to.
    ///
    /// Beside the levels because it is what makes them readable: a bar at zero
    /// is somebody not talking on a channel that is on, and nothing at all on a
    /// channel that is off. Taken from the recorder rather than from settings —
    /// a switch flipped mid-meeting belongs to the next recording, not this one.
    pub channels: ChannelSelection,
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

    // The only path that writes the whole settings row, so it is the only place
    // the vocabulary can be bounded — and it has to be bounded here rather than
    // where it is used, because what is stored is also what the drawer reads
    // back and what the WebView could otherwise grow without limit.
    settings.hot_words = crate::domain::vocabulary::normalise(&settings.hot_words);

    // Cleaned here rather than trusted, because these are copied onto every
    // meeting created from now on and travel from there into model prompts and
    // exported documents. Blank clears, which is what leaves a new meeting
    // reading in the app's own words.
    settings.default_speaker_me = settings
        .default_speaker_me
        .as_deref()
        .and_then(clean_speaker_name);
    settings.default_speaker_others = settings
        .default_speaker_others
        .as_deref()
        .and_then(clean_speaker_name);

    // The theme is not this command's to write. `set_theme` owns it, and the
    // drawer's draft carries whatever the theme was when it opened — so a Save
    // of some unrelated field would put that stale value back and silently undo
    // a theme picked in between. One writer, and the incoming value is ignored
    // rather than validated.
    settings.theme = state.settings.lock().theme.clone();
    // Nor the record shortcut, for a stronger version of the same reason. It is
    // a system-wide key grab: written through here it would skip the allow-list
    // AND the registration, so the row could name a combination the OS never
    // agreed to and the next launch would unregister everything to ask for it.
    // `set_record_shortcut` is the only writer.
    settings.record_shortcut = state.settings.lock().record_shortcut.clone();
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

/// What this meeting calls its two channels, everywhere they are named.
///
/// Read from the meeting and not from settings: the defaults were copied onto
/// the row when it was created, and reading them live would make a rename in
/// the drawer rewrite the transcript of every meeting already recorded.
fn speaker_names(state: &AppState, meeting: &MeetingRecord) -> SpeakerNames {
    SpeakerNames::resolve(
        meeting.speaker_me.as_deref(),
        meeting.speaker_others.as_deref(),
        state.settings.lock().locale(),
    )
}

/// A meeting's segments, warm cache first — the order `get_transcript` answers
/// in. Empty when the meeting has none and when they could not be read: both
/// mean the caller has to fall back to whatever text it already holds.
fn segments_of(state: &AppState, id: &str) -> LiveTranscript {
    let cached = state.live.lock().get(id).cloned();
    match cached {
        Some(t) => t,
        None => state.db.load_transcript(id).unwrap_or_default(),
    }
}

/// The meeting's lines for a model prompt, under the names it goes by now.
///
/// Rendered from the segments rather than read from `transcript_text`, which is
/// a projection frozen at whatever the last write knew: for a meeting that named
/// neither channel it carries the language of that moment, so switching the app
/// to Portuguese would leave the window and the export saying "Eu" while every
/// prompt still read "Me". The fallback belongs to the read, not to the row.
///
/// The stored copy is the last resort rather than the first: a meeting whose
/// segments are gone has nothing else left of its words.
fn prompt_transcript(state: &AppState, meeting: &MeetingRecord) -> String {
    let transcript = segments_of(state, &meeting.id);
    if transcript.segments().is_empty() {
        return meeting.transcript_text.clone();
    }
    transcript.plain_text(&speaker_names(state, meeting))
}

/// The same meeting with its clock, for the one thing that needs it.
///
/// Only summarising. Every action item is asked to end with the moment it was
/// decided and can only copy a timestamp it was given — but the title and the
/// chat read `prompt_transcript` and must go on reading the plain one. Chat
/// truncates at a fixed number of characters and the title gets a fixed
/// excerpt, so a stamp a line there would push the end of a long meeting out of
/// the window and buy nothing.
///
/// A meeting with no segments falls back to the stored text, which has no clock
/// in it. A summary without citations is the honest outcome there: the prompt
/// asks the model to leave the brackets off when the transcript does not show
/// when something came up, and here it shows nothing.
fn summary_transcript(state: &AppState, meeting: &MeetingRecord) -> String {
    let transcript = segments_of(state, &meeting.id);
    if transcript.segments().is_empty() {
        return meeting.transcript_text.clone();
    }
    transcript.timestamped_text(&speaker_names(state, meeting))
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

/// Where this meeting's recording is, once it is proven to be one of ours.
///
/// The window asks by id and never by path, because `audio_path` is a string in
/// a row on the user's disk and the WebView is the other side of the trust
/// boundary. What comes back is a file the asset protocol has just been told to
/// serve — the scope in `tauri.conf.json` is empty, and this is the only thing
/// that ever adds to it, one recording at a time. Naming any other file in the
/// URL is refused by Tauri, which resolves symlinks again on every request, so
/// the grant does not survive the file being swapped for a link pointing out.
///
/// What that does not close is the instant between Tauri's check and its open.
/// No path-based validation closes it — reading the bytes here instead would
/// carry the same race across a wider gap — and both ends of it need somebody
/// who can already write inside the app's data directory, beside the database.
///
/// `None` for every way there is nothing to play: an imported meeting whose
/// audio was not retained, a file deleted from under the app, a row pointing
/// outside the recordings directory, and a meeting still being recorded. The
/// window shows the same thing for all of them; only the row pointing outside
/// gets a line in the log.
#[tauri::command]
pub fn meeting_audio_path(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Option<String>, String> {
    // Said here now, rather than left to the file not existing yet. The
    // recording is written as it is captured, so the WAV of a meeting in
    // progress is on disk from the first second — and handing that to the
    // player would put a transport under a meeting nobody has finished, over a
    // file that is still growing underneath it.
    if state.active_meeting.lock().as_deref() == Some(id.as_str()) {
        return Ok(None);
    }
    let Some(stored) = state.db.get_meeting(&id)?.and_then(|m| m.audio_path) else {
        return Ok(None);
    };
    match playable_recording(&recordings_dir(), &stored) {
        Ok(path) => {
            app.asset_protocol_scope()
                .allow_file(&path)
                .map_err(|e| e.to_string())?;
            Ok(Some(path.display().to_string()))
        }
        Err(Unplayable::Missing) => Ok(None),
        Err(Unplayable::Outside) => {
            // The path is not repeated: it arrived from a row this refuses to
            // trust and this line lands in a file on the user's disk.
            tracing::warn!("refusing to play a recording stored outside the app directory");
            Ok(None)
        }
    }
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
                        at_ms: None,
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
        at_ms: None,
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

    // The search index is built from `transcript_text`, not from the segments,
    // so the correction has to land there or the meeting would stay findable by
    // what was misheard and unfindable by what was said. All three writes —
    // segments, that field, and the index — commit together or not at all.
    meeting.transcript_text = transcript.plain_text(&speaker_names(&state, &meeting));
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
    let (key, offline) = {
        let s = state.settings.lock();
        (
            s.openrouter_api_key.clone().unwrap_or_default(),
            s.offline_mode,
        )
    };
    // The built-in list rather than a refusal: this only fills a dropdown, and
    // an empty picker with an error beside it is a worse answer than the names
    // that ship with the app. The request itself carries the API key, which is
    // exactly the kind of quiet egress the switch exists to stop.
    if offline || key.is_empty() || key.contains('…') {
        return Ok(default_stt_models());
    }
    fetch_openrouter_stt_models(&key).await
}

#[tauri::command]
pub async fn list_openrouter_llm_models(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<OrModel>, String> {
    let (key, offline) = {
        let s = state.settings.lock();
        (
            s.openrouter_api_key.clone().unwrap_or_default(),
            s.offline_mode,
        )
    };
    // The built-in list rather than a refusal: this only fills a dropdown, and
    // an empty picker with an error beside it is a worse answer than the names
    // that ship with the app. The request itself carries the API key, which is
    // exactly the kind of quiet egress the switch exists to stop.
    if offline || key.is_empty() || key.contains('…') {
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
        // Copied into the capture like the device ids beside it, so the
        // selection this meeting was started with is the one it keeps.
        ChannelSelection::from_settings(&settings),
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
        sections: Vec::new(),
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
        // Copied, not referenced. From here this meeting carries its own names,
        // so changing the defaults before the next call leaves this one alone.
        speaker_me: settings.default_speaker_me.clone(),
        speaker_others: settings.default_speaker_others.clone(),
        brief: None,
    };
    state.db.upsert_meeting(&meeting)?;
    state.live.lock().insert(id.clone(), LiveTranscript::new());
    // This meeting's own pair, under this meeting's id. 16 kHz until capture
    // opens and says otherwise — `set_sample_rate` on the first drain is what
    // makes the milliseconds real.
    state
        .segmenters
        .lock()
        .insert(id.clone(), (Segmenter::new(16_000), Segmenter::new(16_000)));
    // A fresh vigil for a fresh clock. Carried over from the last recording, an
    // unanswered question would stop this one within seconds of it starting.
    let mut vigil = state.vigil.lock();
    state.last_speech_ms.store(0, Ordering::SeqCst);
    *vigil = crate::domain::silence::Vigil::default();
    drop(vigil);
    // A fresh pair of ears for a fresh recording. The latch is cleared by the
    // recorder in `start` and the watcher's own clock is local to it; this is
    // the half the window is holding.
    *state.warned_deaf.lock() = crate::domain::deaf::Deaf::default();
    let _ = app.emit("recording://deaf", crate::domain::deaf::Deaf::default());
    // Said out loud rather than left to the window's own state. The question is
    // raised and taken down by events, and an event lost to a race — a stop, a
    // window reload — would otherwise leave it on screen over a meeting it was
    // never asked about.
    let _ = app.emit("recording://silent", false);
    // Started here rather than by the window, so transcription keeps running when
    // the window is minimized and its timers are throttled to a crawl.
    spawn_live_stt_ticker(app.clone(), Arc::clone(&state));
    // After it, and only after: the ticker is what bumps the generation both of
    // them are retired by, so starting this first would leave it comparing
    // against a number that is already stale and stop it on its first tick.
    spawn_recording_flush(Arc::clone(&state));
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
    // The boundary is not marked here. Pressing Pause does end the sentence, but
    // the recorder still holds samples captured before it, and sealing now would
    // put those on the far side of the break — leaving them to be merged with
    // whatever is said after the resume. `drive_live_stt` seals instead, once a
    // drain comes back empty, which is the first moment everything captured
    // before the pause is in the segmenter.
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
        channels: state.recorder.channels(),
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
    // Here rather than in the window, because the window is not the only thing
    // that stops a recording: the overlay, the shortcut and the tray all reach
    // this command directly. A question about a meeting that has ended has to
    // go with it, or it is waiting on screen when the next one starts.
    *state.vigil.lock() = crate::domain::silence::answered();
    let _ = app.emit("recording://silent", false);
    // Here for the same reason the line above is here: the overlay, the
    // shortcut and the tray all stop a recording without going through the
    // window, and a banner about audio that is no longer being captured would
    // stay on screen with nothing left to take it down.
    *state.warned_deaf.lock() = crate::domain::deaf::Deaf::default();
    let _ = app.emit("recording://deaf", crate::domain::deaf::Deaf::default());
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
    // Taken past that await, not before it. A live pass still transcribing holds
    // that lock, so by here it has finished and put back whatever it could not
    // transcribe — into this meeting's own entry, which is what the map is keyed
    // for. Removed rather than borrowed: this recording is over, and nothing may
    // add to it after this point.
    let mut segmenters = state.segmenters.lock().remove(&id);

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
        // Through the segmenters, and then emptied. They are holding the
        // sentence that had not reached a pause yet, and the last sentence of a
        // meeting never does — it is followed by somebody pressing Stop.
        // `None` only if this meeting never had an entry — a recording that
        // started before this build, or one whose start failed after claiming.
        // The tail is still transcribed, just without a buffer in front of it.
        let (mine, theirs) = match segmenters.as_mut() {
            Some(seg) => {
                seg.0.set_sample_rate(sr);
                seg.1.set_sample_rate(sr);
                let mut mine = seg.0.push(&mic);
                let mut theirs = seg.1.push(&sys);
                mine.extend(seg.0.flush());
                theirs.extend(seg.1.flush());
                (mine, theirs)
            }
            None => {
                let mut fresh = (Segmenter::new(sr), Segmenter::new(sr));
                let mut mine = fresh.0.push(&mic);
                let mut theirs = fresh.1.push(&sys);
                mine.extend(fresh.0.flush());
                theirs.extend(fresh.1.flush());
                (mine, theirs)
            }
        };
        if !mine.is_empty() || !theirs.is_empty() {
            // `Skip`, not `StopAndReturn`: there is no later pass to put an
            // utterance back for, so stopping at the first failure would throw
            // away every utterance behind it untried. Out of order is fine here
            // — `apply_stt_chunks` files each one by its own timestamp.
            let (me, others) = tokio::join!(
                run_utterances(&state, &settings, &id, Speaker::Me, mine, sr, OnError::Skip),
                run_utterances(
                    &state,
                    &settings,
                    &id,
                    Speaker::Others,
                    theirs,
                    sr,
                    OnError::Skip
                ),
            );
            let mut chunks = me.0;
            chunks.extend(others.0);
            // Same reason as the summary above: the error names the model, so it
            // is logged rather than shown.
            if me.2.is_some() || others.2.is_some() {
                tracing::warn!("an utterance of the last chunk could not be transcribed");
            }
            if !chunks.is_empty() {
                bill_chunks(&state.db, &id, &chunks);
                apply_stt_chunks(&mut live, &chunks);
            }
        }
    }
    state.live.lock().insert(id.clone(), live.clone());
    let names = speaker_names(&state, &meeting);
    state.db.save_transcript(&id, &live, &names)?;
    meeting.transcript_text = live.plain_text(&names);
    meeting.status = meeting
        .status
        .transition(MeetingEvent::TranscribeDone)
        .map_err(|e| e.to_string())?;
    state.db.upsert_meeting(&meeting)?;

    // The whole recording, read again from the beginning.
    //
    // Where it sits with respect to `stt_flight`: inside it. This command took
    // that lock above and holds it to the end, and it is what keeps a wipe, an
    // import or a retranscribe from writing these same rows underneath the pass
    // — the live ticker is already retired by generation, so nothing else is
    // competing for the transcript. A recording started meanwhile does queue
    // behind this rather than transcribing live, and that is the trade taken
    // knowingly: its audio stays in the recorder and comes out as one backlog
    // when the lock frees, whereas a transcript two writers disagreed about
    // cannot be recovered at all.
    //
    // And deliberately AFTER the transcript is saved and the row has reached
    // `Ready`. Whisper over an hour of audio is minutes of work, and a machine
    // that loses power inside that window must still find a complete transcript
    // and a meeting nothing is waiting to move on: this improves a finished
    // meeting rather than being a step it can get stuck before. That is also why
    // the whole-file fallback above is left in place rather than skipped when
    // this is going to run — it costs a second reading of a recording nobody
    // spoke in, and it is what stands if this one fails.
    if settings.wants_final_stt_pass() {
        let _ = app.emit(
            "meeting://progress",
            &MeetingProgress::new(&id, MeetingPhase::FinalPass),
        );
        let replaced = match final_stt_pass(&state, &settings, &path).await {
            // Replacement is the whole point — see `LiveTranscript::replace_with`
            // for why the two passes are not merged. A pass that came back with
            // nothing leaves the live transcript exactly where it was.
            Ok(better) => live.replace_with(better),
            // Told to the window and not to the log, the same way a failed
            // summary is: the message can name the model id, which arrives from
            // the WebView, and the log is a file on the user's disk. Nothing
            // else changes — the live transcript is saved and the meeting is
            // ready, so this is an improvement that did not arrive rather than
            // a recording that was lost.
            Err(e) => {
                tracing::warn!("the final transcription pass failed");
                let _ = app.emit(
                    "meeting://progress",
                    &MeetingProgress::final_pass_failed(&id, &e),
                );
                false
            }
        };
        // The row is read back rather than the copy this command has been
        // holding since before the pass being written over it. Minutes can have
        // gone by, and `rename_meeting` neither waits for this nor is blocked
        // while it runs — writing the snapshot would take the name the user
        // typed in that time straight back off the meeting.
        //
        // `None` is a meeting deleted while the pass was decoding, and it ends
        // the walk here whichever way the pass went. Everything past this point
        // writes to that row — the transcript, the summary, its cost, the record
        // handed to the window — and `upsert_meeting` would put the meeting back
        // on screen. The summary is the worse half: it would send the transcript
        // of a meeting somebody deleted to whatever model is configured, and on
        // a cloud provider that is a recording leaving the machine after the
        // user asked for it to be gone.
        //
        // A terminal phase still goes out, or the header narrates a re-read that
        // nothing will ever finish. Not `meeting://ready` though: that one puts
        // the record back in the sidebar, which is the thing being avoided.
        let Some(mut fresh) = state.db.get_meeting(&id)? else {
            let _ = app.emit(
                "meeting://progress",
                &MeetingProgress::new(&id, MeetingPhase::Ready),
            );
            return Ok(meeting);
        };
        if replaced {
            // From the row just read, not from the copy held since before the
            // pass: a rename during the pass is exactly what that re-read is
            // for, and the flattened column has to carry the same names the
            // screen does.
            let names = speaker_names(&state, &fresh);
            fresh.transcript_text = live.plain_text(&names);
            fresh.updated_at = chrono::Utc::now().to_rfc3339();
            // Status untouched: it is already `Ready`, and the upsert is what
            // rebuilds the search index over the new words.
            //
            // Failure is not propagated, unlike the identical pair further up.
            // By here the recording is saved, the transcript is stored and the
            // row is `Ready`; the way these fail is a delete landing in the
            // moment between the read above and the write, and reporting that as
            // an error would tell somebody their meeting failed to stop because
            // they deleted it. Nothing is lost either way — what could not be
            // written is an improvement to a meeting that is gone.
            match state
                .db
                .save_transcript(&id, &live, &names)
                .and_then(|()| state.db.upsert_meeting(&fresh))
            {
                Ok(()) => {
                    state.live.lock().insert(id.clone(), live.clone());
                    // Carried forward, so the summary below and the record the
                    // window receives are the row that was just written —
                    // including a title renamed while the pass was running.
                    meeting = fresh;
                }
                Err(e) => tracing::warn!("the re-read transcript could not be stored: {e}"),
            }
        }
    }

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
                // The same stamped text the manual path reads, for the same
                // reason: `transcript_text` has no clock in it, and an action
                // item cannot cite a timestamp it was never shown.
                &summary_transcript(&state, &meeting),
                SummaryTemplate::General,
                &state.db.list_context_notes(&id).unwrap_or_default(),
                // Read now, not from `meeting`. That record was captured before
                // transcription began and the user can write context at any
                // point up to this call — reading its copy drops a brief that
                // is already stored.
                state.db.get_brief(&id).unwrap_or_default().as_deref(),
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
                // Carried onto the record because `upsert_meeting` is what
                // rebuilds the search index, and it indexes what the record
                // holds — a stale list here means a section nobody can find.
                meeting.sections = insights.sections.clone();
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

/// Transcribe the saved recording from the beginning, both channels.
///
/// A separate job over the whole file, not a bigger tail: it reads what is on
/// disk rather than what is left in the recorder, and the engine keeps its own
/// decoding context across the meeting instead of restarting at every pause the
/// segmenter found. Nothing is billed here — the pass is local by construction,
/// which is what `wants_final_stt_pass` decides.
async fn final_stt_pass(
    state: &Arc<AppState>,
    settings: &AppSettings,
    path: &std::path::Path,
) -> Result<LiveTranscript, String> {
    let (mic, sys, sr) = read_dual_wav(path).map_err(|e| e.to_string())?;
    let chunks = state
        .stt
        .transcribe_whole_dual(settings, mic, sys, sr)
        .await?;
    let mut t = LiveTranscript::new();
    apply_stt_chunks(&mut t, &chunks);
    Ok(t)
}

/// Cadence of the backend live-STT ticker.
const LIVE_STT_INTERVAL_MS: u64 = 1200;

/// How often the recording is written to disk.
///
/// The same 1200ms as the live-STT tick, because that is the interval the
/// trade was decided at: never more than that much speech held only in memory.
/// It is a constant of its own because the two run on separate clocks — see
/// `spawn_recording_flush`.
const RECORDING_FLUSH_INTERVAL_MS: u64 = 1200;

/// How much of the previous utterance is handed to the model as context.
///
/// whisper decodes each call from nothing unless told otherwise, which is why
/// the same name came back spelled three ways across three chunks. The tail of
/// what was just said is the cheapest fix there is — no state to keep, because
/// the transcript already holds it.
const PROMPT_TAIL_CHARS: usize = 200;

/// The last `PROMPT_TAIL_CHARS` of a line.
fn prompt_tail(text: &str) -> String {
    let text = text.trim();
    // On a character boundary, not a byte one: this is Portuguese as often as
    // English, and slicing an accented letter in half panics.
    let start = text
        .char_indices()
        .rev()
        .nth(PROMPT_TAIL_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(0);
    text[start..].to_string()
}

/// The end of what this speaker last said, for the model to continue from.
fn context_tail(state: &Arc<AppState>, id: &str, speaker: Speaker) -> String {
    let live = state.live.lock();
    let Some(t) = live.get(id) else {
        return String::new();
    };
    let Some(last) = t.segments().iter().rev().find(|s| s.speaker == speaker) else {
        return String::new();
    };
    prompt_tail(&last.text)
}

/// What a failed utterance means for the ones behind it.
#[derive(Clone, Copy, PartialEq)]
enum OnError {
    /// Give up the rest of the channel and hand it back for the segmenter.
    ///
    /// For the live pass. `put_back` winds the buffer to where the failed
    /// utterance began, so transcribing the ones behind it first would leave the
    /// segmenter's clock ahead of its own audio.
    StopAndReturn,
    /// Carry on with the next one and lose only this.
    ///
    /// For stop, where nothing can be put back because there is no later pass —
    /// so stopping would discard every utterance behind the failure untried.
    Skip,
}

/// Transcribe one channel's utterances, oldest first.
///
/// Returns what landed, what must go back to the segmenter, and the first error.
async fn run_utterances(
    state: &Arc<AppState>,
    settings: &AppSettings,
    id: &str,
    speaker: Speaker,
    utterances: Vec<Utterance>,
    sample_rate: u32,
    on_error: OnError,
) -> (Vec<SttChunkResult>, Vec<Utterance>, Option<String>) {
    let mut done = Vec::new();
    let mut failed = None;
    let mut prompt = context_tail(state, id, speaker);
    let mut left = utterances.into_iter();
    for u in left.by_ref() {
        match state
            .stt
            .transcribe_channel(settings, speaker, &u.pcm, sample_rate, u.start_ms, &prompt)
            .await
        {
            Ok(chunk) => {
                if !chunk.text.is_empty() {
                    // Trimmed like the one read from the transcript. A backlog
                    // chains several utterances through here, and handing each
                    // the whole of the last would grow the prompt without bound
                    // — fifteen seconds of speech is a lot of characters.
                    prompt = prompt_tail(&chunk.text);
                }
                done.push(chunk);
            }
            Err(e) => {
                if on_error == OnError::StopAndReturn {
                    let mut back = vec![u];
                    back.extend(left);
                    return (done, back, Some(e));
                }
                failed.get_or_insert(e);
            }
        }
    }
    (done, Vec::new(), failed)
}

/// Drain whatever audio has arrived and transcribe it.
///
/// No longer a command. It used to be driven by a `setInterval` in the window,
/// and WebView2 treats a minimized window as a hidden page: its timers clamp to
/// roughly one a second and then to one a minute after five. Since this call is
/// what *drains* the recorder, that did not merely slow the display down — it
/// stalled transcription itself for anyone who minimized the app during a
/// meeting, which is precisely when they would.
async fn drive_live_stt(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    if !state.recorder.is_recording() {
        return Ok(());
    }
    // A pause no longer stops this pass — it changes what it is for. The
    // recorder still holds whatever it captured before the button, and that
    // audio has to reach the segmenter before the boundary is marked, or the end
    // of the sentence lands after the break and merges with what is said on the
    // far side of it.
    let paused = state.recorder.is_paused();
    // Single-flight. The UI polls every 1200ms and a chunk can take longer than
    // that to transcribe, so without this the next poll drains a second slice of
    // audio while the first is still running. Both then timestamp their slice from
    // whatever `elapsed_ms` reads at drain time, and the transcript comes out in
    // the wrong order. Overlapping polls now just return what is already there;
    // the audio stays in the buffer for the next turn.
    let Ok(_flight) = state.stt_flight.try_lock() else {
        return Ok(());
    };
    // Read past the lock, never before it. A stop and a start can both land while
    // this pass waits for the flight, and an id taken beforehand would be the old
    // meeting's while the recorder underneath has already become the new one's —
    // so the drain below would take the new meeting's opening seconds and file
    // them, or on failure buffer them, against the meeting that just ended.
    let id = state
        .active_meeting
        .lock()
        .clone()
        .ok_or_else(|| "no active meeting".to_string())?;
    let (mic, sys, sr) = state.recorder.drain_chunks();
    if mic.is_empty() && sys.is_empty() {
        // Empty while paused is the moment the boundary becomes safe: everything
        // captured before the button is in the segmenter, and nothing more is
        // coming until the resume. Idempotent, so the ticks that follow while
        // the recording sits paused cost a flag write and nothing else.
        //
        // Bounded, and knowingly: a pause and a resume that both land inside one
        // 1200ms tick are never observed here, so that utterance spans the
        // break. Closing it needs a mark inside the captured stream, since a
        // drain after such a resume carries both sides in one buffer with
        // nothing between them. Not a regression — cutting on the clock spanned
        // the break too — and a one-second pause is not a boundary anybody means.
        if paused {
            if let Some(seg) = state.segmenters.lock().get_mut(&id) {
                seg.0.seal();
                seg.1.seal();
            }
        }
        return Ok(());
    }
    // The drain is no longer the unit of transcription. What comes out of the
    // recorder goes into the segmenters, and only a whole utterance — bounded by
    // a pause — is sent to a model. A poll that lands mid-sentence now adds to
    // the buffer instead of cutting the word in half.
    let (mine, theirs) = {
        let mut all = state.segmenters.lock();
        // Looked up, never inserted. A ticker that outlives its stop by a beat
        // would otherwise create an entry nobody is left to flush, and the audio
        // in it would sit there until the process ended. The recorder takes its
        // samples back instead, so nothing is dropped on the way out.
        let Some(seg) = all.get_mut(&id) else {
            state.recorder.rewind_chunks(mic.len(), sys.len());
            return Ok(());
        };
        seg.0.set_sample_rate(sr);
        seg.1.set_sample_rate(sr);
        (seg.0.push(&mic), seg.1.push(&sys))
    };
    if mine.is_empty() && theirs.is_empty() {
        return Ok(());
    }
    let settings = state.settings.lock().clone();
    // The two channels are independent, so waiting for one before starting the
    // other would double the latency of every utterance for no reason.
    let (me, others) = tokio::join!(
        run_utterances(
            state,
            &settings,
            &id,
            Speaker::Me,
            mine,
            sr,
            OnError::StopAndReturn
        ),
        run_utterances(
            state,
            &settings,
            &id,
            Speaker::Others,
            theirs,
            sr,
            OnError::StopAndReturn
        ),
    );
    // Put back before propagating, and in reverse so the oldest ends up at the
    // head. Audio dropped here is a slice of a meeting nobody can get back — the
    // reason the recorder already rewinds its own cursor when a provider rejects
    // a chunk.
    {
        let mut all = state.segmenters.lock();
        if let Some(seg) = all.get_mut(&id) {
            for u in me.1.into_iter().rev() {
                seg.0.put_back(u);
            }
            for u in others.1.into_iter().rev() {
                seg.1.put_back(u);
            }
        }
    }
    let mut chunks = me.0;
    chunks.extend(others.0);
    if let Some(e) = me.2.or(others.2) {
        // Whatever did land is still merged below on the next poll; this pass
        // reports the failure so the window can say transcription is failing.
        if chunks.is_empty() {
            return Err(e);
        }
        tracing::warn!("an utterance could not be transcribed: {e}");
    }
    if chunks.is_empty() {
        return Ok(());
    }
    // Charged before the transcript is merged. The sum goes through SQL rather
    // than a read-modify-write here: two channels transcribe concurrently and
    // one would overwrite the other.
    bill_chunks(&state.db, &id, &chunks);
    // Words, not merely a pass. A chunk can come back empty — a cloud provider
    // charges for the seconds it listened to either way — and treating that as
    // speech would mean the quiet is never noticed at all.
    if chunks.iter().any(|c| !c.text.trim().is_empty()) {
        state
            .last_speech_ms
            .store(state.recorder.elapsed_ms(), Ordering::SeqCst);
    }
    state.live_stt_passes.lock().insert(id.clone());
    let mut guard = state.live.lock();
    let t = guard.entry(id.clone()).or_default();
    apply_stt_chunks(t, &chunks);
    let snapshot = t.clone();
    drop(guard);
    let _ = app.emit("transcript://append", &snapshot);
    Ok(())
}

/// Write the recording to disk for as long as this recording lasts.
///
/// Its own thread, and not a line inside the live-STT ticker where this began.
/// That loop awaits a transcription before it comes back round, and a model
/// answering in four seconds would make this a four-second tick — so the audio
/// held only in memory would be bounded by however long the slowest chunk took,
/// which is not a bound at all. A clock of its own is the only way the interval
/// above is the interval.
///
/// A plain thread rather than a task, because the write blocks: it belongs
/// somewhere blocking is what the thread is for, not on the async runtime the
/// rest of the app shares.
///
/// Retired by generation like the live ticker, and by the same reasoning — a
/// stop followed quickly by a start must not leave two of these writing into
/// one file.
fn spawn_recording_flush(state: Arc<AppState>) {
    let generation = state.live_stt_generation.load(Ordering::SeqCst);
    let _ = std::thread::Builder::new()
        .name("vesper-flush".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(
                RECORDING_FLUSH_INTERVAL_MS,
            ));
            if state.live_stt_generation.load(Ordering::SeqCst) != generation
                || !state.recorder.is_recording()
            {
                // Nothing is lost by stopping here: `stop` writes whatever
                // these ticks did not reach before it hands the file over.
                break;
            }
            state.recorder.flush_to_disk();
        });
}

/// Drive live transcription from the backend for as long as this recording lasts.
///
/// Retired by generation rather than by a stop flag: a stop followed quickly by a
/// start would otherwise leave two tickers alive, splitting each window of audio
/// between them and running concurrent transcriptions over halves of the same
/// speech. The check runs before the work, never mid-flight, so a pass already
/// running still lands.
/// Watch for a dead input on a loop of its own.
///
/// Not on the live-STT ticker, and the reason is the promise this makes. That
/// loop awaits a model before it comes round again — seconds for a local one,
/// up to three minutes for a cloud provider — so a warning riding on it would
/// arrive whenever transcription happened to finish, or never, if the user
/// stopped first. A twenty-second promise cannot be kept behind an unbounded
/// wait, and the failure this warns about is the one where the microphone is
/// working, which is exactly when that wait is longest.
///
/// Retired by the same generation counter as the ticker: a stop followed
/// quickly by a start would otherwise leave two of these alive, both reading one
/// recorder.
const DEAF_POLL_MS: u64 = 1_000;

fn spawn_deaf_watch(app: AppHandle, state: Arc<AppState>, generation: u64) {
    tauri::async_runtime::spawn(async move {
        // This loop's own clock, and it is deliberately not the recorder's.
        // `elapsed_ms` races `pause`, which sets its flag before it takes the
        // recorder lock and adds the interval — a reader landing in that gap
        // gets a stale elapsed and would start a countdown from a moment that
        // had already gone by. Counting ticks that actually ran, and skipping
        // the paused ones, needs no such reading and cannot be stale.
        let mut watched_ms = 0u64;
        // `watched_ms` when a channel was first seen to have heard something.
        // Local to this recording's watcher, so a stop and a quick start cannot
        // carry it over.
        let mut evidence_ms: Option<u64> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(DEAF_POLL_MS)).await;
            if state.live_stt_generation.load(Ordering::SeqCst) != generation
                || !state.recorder.is_recording()
            {
                break;
            }
            // Paused time is not recording time, and a meeting somebody paused
            // for ten minutes has not been failing to hear anything for ten
            // minutes.
            if state.recorder.is_paused() {
                continue;
            }
            watched_ms += DEAF_POLL_MS;
            watch_for_deaf_channels(&app, &state, generation, watched_ms, &mut evidence_ms);
        }
    });
}

fn spawn_live_stt_ticker(app: AppHandle, state: Arc<AppState>) {
    let generation = state.live_stt_generation.fetch_add(1, Ordering::SeqCst) + 1;
    spawn_deaf_watch(app.clone(), Arc::clone(&state), generation);
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
            watch_for_silence(&app, &state);
        }
    });
}

/// The user answered the "still there?" question.
///
/// Whatever they clicked, they are there. The clock of quiet restarts too, not
/// only the state — otherwise the next tick would find three minutes of silence
/// still on the counter and ask again immediately.
/// Picks that were moved off a model the catalogue no longer offers.
///
/// Drained: the window asks once at startup and shows what it gets, and a
/// notice repeated at every launch about a change made months ago is noise.
/// Nothing else reads this, so taking it is not losing it.
#[tauri::command]
pub fn retired_models(state: State<'_, Arc<AppState>>) -> Vec<(String, String)> {
    std::mem::take(&mut *state.retired_models.lock())
}

#[tauri::command]
pub fn keep_recording(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Both under the vigil lock, which guards the pair: the watcher reads the
    // clock under it too, and an answer landing between the two would leave the
    // state reset and the clock stale — the question raised again on the spot,
    // at the user who just said to keep recording.
    let mut vigil = state.vigil.lock();
    state
        .last_speech_ms
        .store(state.recorder.elapsed_ms(), Ordering::SeqCst);
    *vigil = crate::domain::silence::answered();
    drop(vigil);
    let _ = app.emit("recording://silent", false);
    Ok(())
}

/// Audio that exists and nobody has read.
///
/// Three sources, and the reason all three count is the same: a recording must
/// never end over audio nobody has looked at. A segmenter holding an utterance
/// is somebody mid-sentence; a segmenter holding one that came back is a
/// provider that refused it and said nothing about the room; and samples the
/// last pass did not reach are somebody who started talking after the drain,
/// which is exactly the person the question was asked about.
fn has_unread_audio(state: &Arc<AppState>) -> bool {
    if state
        .segmenters
        .lock()
        .values()
        .any(|(mine, theirs)| mine.holding() || theirs.holding())
    {
        return true;
    }
    state.recorder.unread_has_speech()
}

/// Notice a room that has gone quiet, and eventually stop recording it.
///
/// Runs on the ticker rather than inside `drive_live_stt`, which returns early
/// on every path where nothing was said — which is every path that matters
/// here. Silence is the absence of the work that function does, so it cannot be
/// the one to spot it.
///
/// A paused recording is not a quiet one. The clock does not advance while
/// paused, so asking whether anybody is there would be asking about a decision
/// the user has already made.
/// Notice a channel that was asked to record and is hearing nothing at all.
///
/// Separate from the silence guard next to it, and the difference is the whole
/// point: that one watches for a room where nobody is speaking, this one
/// watches for a channel that is not connected to a room. The first is about
/// people, and its answer is to stop; the second is about hardware, and its
/// answer is to say so immediately, because the recording is being lost right
/// now and the user is the only one who can fix it.
///
/// The evidence is the level meter rather than transcribed words. A meter is a
/// poor witness for "somebody spoke" — a fan moves it — but an excellent one
/// for "this input is dead", which is the only question asked here.
fn watch_for_deaf_channels(
    app: &AppHandle,
    state: &Arc<AppState>,
    generation: u64,
    watched_ms: u64,
    evidence_ms: &mut Option<u64>,
) {
    use crate::domain::deaf::deaf_channels;

    // From the recorder's own latch, not from the level meter. The meter holds
    // the most recent chunk and is replaced every 20ms, so reading it here would
    // sample one chunk in fifty and decide a channel was dead over the ones it
    // never saw.
    let (heard_me, heard_others) = state.recorder.heard();
    let channels = state.recorder.channels();
    // The first tick that sees either latch set is when this recording got its
    // evidence. Written once — a later tick must not push the deadline back.
    if evidence_ms.is_none() && (heard_me || heard_others) {
        *evidence_ms = Some(watched_ms);
    }
    let deaf = deaf_channels(
        watched_ms,
        channels.me,
        channels.others,
        heard_me,
        heard_others,
        *evidence_ms,
    );
    // Only on a change, and only ever towards worse. A channel that starts
    // working mid-meeting takes its own warning down; one that has already been
    // reported does not report itself again on the next tick.
    let mut said = state.warned_deaf.lock();
    // Under the lock, and last. `stop_recording` clears this and emits the
    // all-clear, and a tick already past its own checks would otherwise put the
    // banner straight back over a recording that has ended. The generation is
    // rechecked with it, because a stop followed quickly by a start leaves this
    // watcher holding a verdict about a meeting that is over.
    if deaf != *said
        && state.recorder.is_recording()
        && state.live_stt_generation.load(Ordering::SeqCst) == generation
    {
        *said = deaf;
        // The lock is held across the emit rather than dropped before it. Let
        // go here and a stop plus a new recording can clear the banner and
        // advance the generation while this thread is still on its way to
        // `emit`, delivering last and painting the old meeting's warning over
        // the new one.
        if deaf.any() {
            tracing::error!(
                "capture is silent — me: {}, others: {}",
                deaf.me,
                deaf.others
            );
        }
        let _ = app.emit("recording://deaf", deaf);
    }
}

fn watch_for_silence(app: &AppHandle, state: &Arc<AppState>) {
    use crate::domain::silence::{advance, Act};

    if state.recorder.is_paused() {
        return;
    }
    let now = state.recorder.elapsed_ms();
    let unread = has_unread_audio(state);
    // One acquisition, and `last_speech_ms` is read under it: the vigil mutex
    // guards the pair. Read outside, an answer landing between the two leaves
    // the state reset and the clock stale, and the question is raised again on
    // the spot — for a user who just said to keep recording.
    let mut vigil = state.vigil.lock();
    let quiet_for = now.saturating_sub(state.last_speech_ms.load(Ordering::SeqCst));
    // Speech is what the last pass found, not a level meter: a fan, a keyboard
    // and a television all move a meter, and none of them is somebody talking.
    let speaking = quiet_for < LIVE_STT_INTERVAL_MS * 2;
    let (next, act) = advance(*vigil, quiet_for, now, speaking, unread);
    *vigil = next;
    drop(vigil);
    match act {
        Act::Ask => {
            let _ = app.emit("recording://silent", true);
        }
        Act::Dismiss => {
            let _ = app.emit("recording://silent", false);
        }
        // The window stops it, through the same command a click goes through.
        // Stopping from here would be a second stop path with none of the
        // guards that one has — the flight lock, the final transcription, the
        // summary — and the two would drift.
        Act::Stop => {
            let _ = app.emit("recording://silent", false);
            let _ = app.emit("recording://stop-silent", ());
        }
        Act::Nothing => {}
    }
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
        .title(settings, summary, &prompt_transcript(state, meeting))
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
    // Before the row is read, not after the status is written. Held later, two
    // overlapping calls both read `Summarizing` — the second one's idea of "what
    // it was before" is the first one's work in progress, and restoring that on
    // an error would put a finished summary back into a state nobody is going
    // to finish.
    //
    // It is also held for the reason it always was: this replaces the whole
    // insight set, and a refinement awaiting its model call would otherwise
    // write a version built from what this is about to overwrite.
    let _flight = state.refine_flight.lock().await;
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    let tpl = SummaryTemplate::from_id(template.as_deref().unwrap_or("general"));
    let settings = state.settings.lock().clone();
    let mut m = meeting;
    let before = m.status;
    m.status = m
        .status
        .transition(MeetingEvent::StartSummarize)
        .unwrap_or(MeetingStatus::Summarizing);
    state.db.upsert_meeting(&m)?;
    let outcome = state
        .llm
        .summarize(
            &settings,
            &summary_transcript(&state, &m),
            tpl,
            // Empty on failure rather than refusing to summarise: a note that
            // cannot be read is a worse summary, not a lost meeting.
            &state.db.list_context_notes(&id).unwrap_or_default(),
            m.brief.as_deref(),
        )
        .await;
    // Put the status back before propagating. `Summarizing` was written to the
    // row a few lines up, and `?` on the call above walked out over it — the
    // meeting then sat in the sidebar summarising forever, through restarts,
    // because nothing was ever going to finish it. Not `Failed` either: the
    // transcript is intact and the meeting is as usable as it was a moment ago,
    // so it goes back to exactly what it was.
    let (insights, cost) = match outcome {
        Ok(v) => v,
        Err(e) => {
            m.status = before;
            // Said out loud rather than swallowed. If this write is the thing
            // that failed — a full disk, a locked database — the row really is
            // stuck in `Summarizing`, which is the exact state this arm exists
            // to prevent, and reporting only the model error would send the
            // user to fix the wrong thing.
            return match state.db.upsert_meeting(&m) {
                Ok(()) => Err(e),
                Err(restore) => Err(format!(
                    "{e} — and the meeting could not be put back: {restore}"
                )),
            };
        }
    };
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
    m.sections = insights.sections.clone();
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
                transcript: &prompt_transcript(&state, &meeting),
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

/// Standing context for one meeting: who was in the room, what it was for.
///
/// Bounded for the same reason a note is — it reaches a prompt, and a megabyte
/// pasted here would push the transcript out of the model's window and quietly
/// ruin the summary. Larger than a note's limit because this is prose about the
/// whole meeting rather than one observation in it.
///
/// Blank is not an error: clearing the field is how a user says there is no
/// context here, and `set_brief` stores that as `NULL`.
#[tauri::command]
pub fn set_meeting_brief(
    state: State<'_, Arc<AppState>>,
    id: String,
    brief: String,
) -> Result<(), String> {
    if brief.chars().count() > 4_000 {
        return Err("that context is too long".into());
    }
    state.db.set_brief(&id, &brief)
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
    // The transcription single-flight, held for the whole job. Not for the
    // recorder's sake — this reads a file — but so a wipe cannot delete the row
    // this is about to write. `wipe_all_cmd` takes the same lock, and without it
    // a confirmed wipe finishes while an import is mid-transcription and the
    // meeting appears afterwards, pointing at a WAV that is already gone.
    let _flight = state.stt_flight.lock().await;
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
    let settings = state.settings.lock().clone();
    let mut meeting = MeetingRecord {
        id: id.clone(),
        title,
        status: MeetingStatus::Transcribing,
        sections: Vec::new(),
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
        // Copied at creation, like a recorded meeting's: an import is a meeting
        // too, and it keeps the names the app was set to when it arrived.
        speaker_me: settings.default_speaker_me.clone(),
        speaker_others: settings.default_speaker_others.clone(),
        brief: None,
    };
    state.db.upsert_meeting(&meeting)?;
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
    let names = speaker_names(&state, &meeting);
    state.db.save_transcript(&id, &t, &names)?;
    meeting.transcript_text = t.plain_text(&names);
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
    // Same reason as `import_audio`: this writes a meeting, so it has to be a
    // thing a wipe waits for rather than a thing that outlives one.
    let _flight = state.stt_flight.lock().await;
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
    let names = speaker_names(&state, &meeting);
    state.db.save_transcript(&id, &t, &names)?;
    meeting.transcript_text = t.plain_text(&names);
    meeting.status = MeetingStatus::Ready;
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    state.db.upsert_meeting(&meeting)?;
    Ok(meeting)
}

/// The meetings the application was recording when it last stopped running.
///
/// Asked once at launch. Nothing here changes anything: the answer is an offer,
/// and until the user takes it the recording stays exactly where it is.
#[tauri::command]
pub fn interrupted_meetings(state: State<'_, Arc<AppState>>) -> Result<Vec<MeetingRecord>, String> {
    let active = state.active_meeting.lock().clone();
    Ok(interrupted(state.db.list_meetings()?, active.as_deref()))
}

/// Finish a meeting whose recording was cut short by a crash or a kill.
///
/// The stop that never ran, in the order that leaves nothing stuck: the row is
/// only moved once the transcript exists. A recovery interrupted in its turn —
/// this reads the whole recording, which on a long meeting is minutes of work —
/// therefore leaves a meeting that is offered again at the next launch, rather
/// than one sitting in `Transcribing` with nothing left to move it on.
#[tauri::command]
pub async fn recover_meeting(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<MeetingRecord, String> {
    // The same lock every other transcription takes, and for the same reason as
    // `retranscribe`: this writes a meeting, so a wipe has to wait for it
    // instead of deleting the row underneath.
    let _flight = state.stt_flight.lock().await;
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    // The id arrives from the WebView, so what it names is checked here rather
    // than assumed from the list the window was given: only a meeting actually
    // waiting to be recovered may be walked through this.
    if !is_interrupted(&meeting, state.active_meeting.lock().clone().as_deref()) {
        return Err("that meeting is not waiting to be recovered".into());
    }
    let stored = meeting
        .audio_path
        .clone()
        .ok_or_else(|| "no audio for meeting".to_string())?;
    // Through the same guard the player uses. A row is a line in a file on the
    // user's disk, and this both rewrites the file it names and reads it out.
    let path = playable_recording(&recordings_dir(), &stored).map_err(|e| match e {
        Unplayable::Missing => "the recording is not on this computer".to_string(),
        Unplayable::Outside => {
            tracing::warn!("refusing to recover a recording stored outside the app directory");
            "the recording is not where it should be".to_string()
        }
    })?;
    // The header was last written a flush before the crash, so what it declares
    // is short of what is in the file. Everything below reads through it.
    repair_wav_header(&path).map_err(|e| e.to_string())?;
    let (mic, sys, sr) = read_dual_wav(&path).map_err(|e| e.to_string())?;

    let _ = app.emit(
        "meeting://progress",
        &MeetingProgress::new(&id, MeetingPhase::Transcribing),
    );
    let settings = state.settings.lock().clone();
    let mut live = LiveTranscript::new();
    // A crash inside the first tick leaves a file with a header and nothing
    // behind it. Skipped rather than sent: a provider charges for the call
    // whatever is in it, and there is nothing here to transcribe.
    if !mic.is_empty() || !sys.is_empty() {
        let chunks = state
            .stt
            .transcribe_dual(&settings, &mic, &sys, sr, 0)
            .await?;
        // Charged as soon as the provider has answered, before the transcript
        // is written. A crash in the gap leaves this meeting to be offered
        // again, and a second recovery pays the provider a second time — but
        // the figure the window shows is then the sum of what was actually
        // spent, which is the property worth keeping. Recording the cost after
        // the transcript would make a crash lose a charge that really happened,
        // and a meeting that quietly under-reports what it cost is the worse of
        // the two. Nothing here retries on its own: the second call only
        // happens because somebody was asked and said yes.
        bill_chunks(&state.db, &id, &chunks);
        apply_stt_chunks(&mut live, &chunks);
    }

    // The row is read again rather than the copy taken before the transcription
    // being written over it. Minutes can have gone by, and nothing stops the
    // user renaming this meeting in that time — writing the snapshot would take
    // the name they typed straight back off it.
    //
    // `None` is a meeting deleted while the pass was decoding, and it ends the
    // walk: `upsert_meeting` would put a meeting somebody removed back on
    // screen, pointing at a recording that is no longer there.
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "that meeting was removed while it was being recovered".to_string())?;
    // The segments first and the status last, which is the order the stop path
    // writes them in and for a sharper reason here: the row leaving `Recording`
    // is what takes this meeting off the list of ones to offer back. A status
    // committed before the transcript would, if the write after it failed, mean
    // a meeting nothing offers to recover and nothing is left to transcribe —
    // the audio still on disk and no way to reach it. Failing before that leaves
    // the meeting exactly as it was, and the next launch asks again.
    let names = speaker_names(&state, &meeting);
    state.db.save_transcript(&id, &live, &names)?;
    // `duration_ms` is stamped by the stop that never happened, so the file's
    // own length is the only record of how long the meeting was.
    meeting.duration_ms = (mic.len().max(sys.len()) as u64 * 1000) / sr.max(1) as u64;
    meeting.status = meeting
        .status
        .transition(MeetingEvent::StopRecording)
        .and_then(|s| s.transition(MeetingEvent::TranscribeDone))
        .map_err(|e| e.to_string())?;
    meeting.updated_at = chrono::Utc::now().to_rfc3339();
    meeting.transcript_text = live.plain_text(&names);
    state.db.upsert_meeting(&meeting)?;
    state.live.lock().insert(id.clone(), live);
    let _ = app.emit(
        "meeting://progress",
        &MeetingProgress::new(&id, MeetingPhase::Ready),
    );
    Ok(meeting)
}

/// Throw away a meeting whose recording was cut short, with its partial audio.
///
/// Guarded the same way as `recover_meeting`, and this is the half where it
/// matters: this deletes a recording, and an id that is stale — a window that
/// asked before a recording started, an offer answered twice — must not be able
/// to reach a meeting the user still has.
///
/// Behind the transcription lock for that reason and not for its own work,
/// which touches no model. A recovery of this same meeting is minutes long and
/// leaves the row saying `Recording` until it finishes; without waiting for it,
/// this would delete the recording out from under a user who had asked to keep
/// it. Past the lock the row has been moved on, the check below refuses, and
/// the answer is that there is nothing left to discard.
#[tauri::command]
pub async fn discard_interrupted_meeting(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<(), String> {
    let _flight = state.stt_flight.lock().await;
    let meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    if !is_interrupted(&meeting, state.active_meeting.lock().clone().as_deref()) {
        return Err("that meeting is not waiting to be recovered".into());
    }
    // The ordinary delete, which removes the recording before the rows so a
    // file that could not be deleted leaves the meeting whole and the discard
    // repeatable.
    state.db.delete_meeting(&id)
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
    let mut insights = MeetingInsights {
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
        // The export writes every section it is given, so a client call leaves
        // with its Requirements and Risks rather than with the three the
        // general template happens to share.
        sections: meeting.sections.clone(),
    };
    // The stored section bodies were frozen when the model answered. The action
    // items have moved since: the merge, the checkbox and every edit write the
    // rows and the column, never the JSON — so exporting the frozen copy would
    // omit the tasks the user added and keep the ones they deleted.
    insights.sync_sections();
    let fmt = ExportFormat::from_ext(&format).ok_or_else(|| "unsupported format".to_string())?;
    export_meeting(
        std::path::Path::new(&path),
        fmt,
        &meeting.title,
        &transcript,
        Some(&insights),
        &speaker_names(&state, &meeting),
    )
}

/// Every meeting, written into a folder the user chose.
///
/// The half of a wipe that has to happen first. Deleting years of meetings is
/// only a reasonable thing to offer if the user can take them with them, and
/// "export each of the two hundred by hand" is not an offer.
///
/// Markdown, and only markdown. It is the one format that is readable without
/// this application, and a PDF of a transcript is a thing you cannot search
/// with the tools someone will actually have in five years.
#[tauri::command]
pub fn export_all_cmd(state: State<'_, Arc<AppState>>, dir: String) -> Result<usize, String> {
    let dir = std::path::Path::new(&dir);
    // The WebView chose this path. It is a folder picker today, and a folder
    // picker is still the WebView — a string that is not a directory would have
    // `join` build file names beside it instead of inside it.
    if !dir.is_dir() {
        return Err("that is not a folder".into());
    }
    let meetings = state.db.list_meetings()?;
    let mut written = 0usize;
    let mut used: HashSet<String> = HashSet::new();
    for m in meetings {
        let transcript = state.db.load_transcript(&m.id)?;
        let mut insights = MeetingInsights {
            summary: m.summary.clone().unwrap_or_default(),
            key_points: bullets(m.key_points.as_deref()),
            action_items: bullets(m.action_items.as_deref()),
            sections: m.sections.clone(),
        };
        // Same as the single-meeting export: the stored section bodies were
        // frozen when the model answered, and the action items have moved since.
        insights.sync_sections();
        // `safe_file_stem`, never the title: it is user text and it reaches a
        // filesystem here.
        //
        // The name has to be free twice over — unused by this run AND absent
        // from the folder. Two meetings can carry the same title; so can a file
        // the user exported last month, and the markdown writer truncates, so
        // the second would replace the first with no sign it had. Compared
        // lowercased because on Windows two names differing only in case are one
        // file.
        let base = safe_file_stem(&m.title);
        let mut stem = base.clone();
        let mut n = 2;
        let file = loop {
            let candidate = dir.join(format!("{stem}.md"));
            // `create_new` rather than `exists`, and it does the reserving as
            // well as the asking. `exists()` follows links, so a dangling
            // `<stem>.md` symlink pointing somewhere else reads as absent and
            // the writer follows it — an export outside the folder the user
            // chose. This fails on a symlink, dangling or not, and fails on a
            // file that appeared between the question and the answer.
            if used.insert(stem.to_lowercase()) {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                {
                    Ok(file) => break file,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(format!("could not write into that folder: {e}")),
                }
            }
            stem = format!("{base} ({n})");
            n += 1;
        };
        // Written through the handle that reserved the name, never reopened by
        // path. Closing it and calling the exporter would leave a gap in which
        // the empty file could be swapped for a symlink, and the reopen would
        // follow it out of the folder the user chose. `build_markdown` is what
        // the single-meeting export writes anyway.
        let md = build_markdown(
            &m.title,
            &transcript,
            Some(&insights),
            &speaker_names(&state, &m),
        );
        {
            use std::io::Write;
            let mut file = file;
            file.write_all(md.as_bytes())
                .map_err(|e| format!("could not write that meeting: {e}"))?;
        }
        written += 1;
    }
    Ok(written)
}

/// Delete every meeting on this computer.
///
/// Transcripts, summaries, notes, tasks, chat and the audio recordings. Not the
/// settings and not the API key: this is the "take my meetings off this machine"
/// button, and a user who presses it still has an application to use afterwards.
///
/// No confirmation here. The window asks, because the window is where a person
/// can be shown what they are about to lose; a command that asked twice would
/// be asking the same WebView that already answered.
#[tauri::command]
pub async fn wipe_all_cmd(state: State<'_, Arc<AppState>>) -> Result<usize, String> {
    // Not while the microphone is open.
    if state.recorder.is_recording() {
        return Err("stop the recording first".into());
    }
    // And not while anything else is still writing a meeting. `is_recording` is
    // already false the moment Stop is pressed, but `stop_recording` goes on for
    // seconds afterwards — final transcription, then the summary — and it holds
    // a `MeetingRecord` it upserts at the end. Wiping in that window deletes the
    // row and the audio, and then that upsert puts the meeting back: the user
    // watches everything vanish and one thing return. Both locks, in the order
    // every other caller takes them.
    let _stt = state.stt_flight.lock().await;
    let _refine = state.refine_flight.lock().await;
    // The recorder's claim, held across the whole delete. `start_recording`
    // takes this before it opens capture and treats it as the mutual exclusion,
    // so holding it is what stops a recording beginning between the check and
    // the delete — a status flag read twice cannot, and the meeting it created
    // would either be deleted while its WAV was still being written or survive
    // a wipe the user had confirmed.
    //
    // An import can hold `stt_flight` for a minute, so this window is not
    // theoretical: it is however long the user waits after pressing Delete.
    let claim = state.active_meeting.lock();
    if claim.is_some() || state.recorder.is_recording() {
        return Err("stop the recording first".into());
    }
    let removed = state.db.delete_all_meetings();
    drop(claim);
    // The in-memory transcripts, whatever the rows did. `get_transcript` answers
    // from this map before it consults the database, so leaving it populated
    // means a deleted meeting's words are still readable until the app restarts
    // — which is the one thing this button exists to prevent.
    //
    // Cleared even when the delete reported a failure: what did get deleted must
    // not stay readable because something else did not.
    state.live.lock().clear();
    state.live_stt_passes.lock().clear();
    state.segmenters.lock().clear();
    removed
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
pub fn shortcut_status(status: State<'_, std::sync::Mutex<ShortcutStatus>>) -> ShortcutStatus {
    status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|e| e.into_inner().clone())
}

/// Register a combination as the record accelerator, replacing whatever held it.
///
/// Shared by startup and by the settings screen so there is one place that
/// knows what the callback does. Unregistering everything first is deliberate:
/// this application owns exactly one accelerator, and leaving the old one live
/// would give a user who changed it two working shortcuts.
pub fn register_record_shortcut(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let parsed: Shortcut = accelerator
        .parse()
        .map_err(|_| format!("{accelerator} is not a combination this build can register"))?;
    let _ = app.global_shortcut().unregister_all();
    let handle = app.clone();
    app.global_shortcut()
        .on_shortcut(parsed, move |_app, _sc, event| {
            if event.state == ShortcutState::Pressed {
                let _ = handle.emit("hotkey://toggle-record", ());
            }
        })
        .map_err(|e| e.to_string())
}

/// Change the record accelerator, and say whether the OS agreed.
///
/// The WebView is the trust boundary and this is a system-wide key grab, so the
/// request is checked against the offered list before anything is parsed —
/// `is_offered`, not a parser over arbitrary input.
///
/// A refusal puts the previous combination back rather than leaving the user
/// with none. That is the case this whole feature exists for: the reason to
/// change a shortcut is that something else already owns it, and the
/// replacement can be owned too.
#[tauri::command]
pub fn set_record_shortcut(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    status: State<'_, std::sync::Mutex<ShortcutStatus>>,
    accelerator: String,
) -> Result<ShortcutStatus, String> {
    if !crate::domain::shortcut::is_offered(&accelerator) {
        return Err("that is not one of the offered combinations".into());
    }
    let previous = { state.settings.lock().record_shortcut.clone() };
    let result = register_record_shortcut(&app, &accelerator);
    let next = if result.is_ok() {
        // Whichever home is holding the API key keeps holding it. Hardcoding
        // `Keychain` here would strip the key from the row on a machine whose
        // keychain refused to cooperate — the one place it is still stored — and
        // changing a keyboard shortcut would silently cost the user their
        // credential. `set_theme` carries the same guard for the same reason.
        let home = if state.key_in_keychain.load(Ordering::Relaxed) {
            KeyHome::Keychain
        } else {
            KeyHome::KeepInRow
        };
        let mut settings = state.settings.lock();
        settings.record_shortcut = accelerator.clone();
        let snapshot = settings.clone();
        drop(settings);
        if let Err(e) = state.db.save_settings_with(&snapshot, home) {
            // The OS already has the new combination and the row does not. Put
            // both back rather than leaving a shortcut that works until the next
            // launch and then reverts with no explanation.
            let restore = crate::domain::shortcut::chosen_or_default(&previous);
            let _ = register_record_shortcut(&app, restore);
            state.settings.lock().record_shortcut = previous;
            return Err(e);
        }
        crate::domain::shortcut::status_from(Ok::<(), String>(()), &accelerator)
    } else {
        // Back to what was working, so a refused change costs the user nothing.
        let restore = crate::domain::shortcut::chosen_or_default(&previous);
        let restored = register_record_shortcut(&app, restore);
        // What is stored is what is ACTIVE. The request failed; the previous
        // combination is the one the OS holds, and the empty state reads this
        // status to tell the user which key works. Storing the rejected one
        // would have every screen naming a key that does nothing.
        let active = crate::domain::shortcut::status_from(restored.as_ref().map(|_| ()), restore);
        if let Ok(mut held) = status.lock() {
            *held = active;
        }
        // Neither one took: something grabbed the previous combination during
        // the moment it was unregistered, so there is no global shortcut at all
        // and saying "the previous one is still in place" would be the opposite
        // of the truth.
        return Err(if restored.is_ok() {
            "shortcut.taken".to_string()
        } else {
            "shortcut.none".to_string()
        });
    };
    if let Ok(mut held) = status.lock() {
        *held = next.clone();
    }
    Ok(next)
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
    let main = app.get_webview_window("main");
    let minimized = main
        .as_ref()
        .and_then(|w| w.is_minimized().ok())
        .unwrap_or(false);
    // Asked of the window rather than remembered from the last event. A focus
    // flag kept in state is one that a missed event leaves wrong forever, and
    // this runs on every resize and focus change anyway.
    //
    // `unwrap_or(true)` — a platform that cannot answer is treated as focused,
    // so the failure mode is a card that does not appear rather than one that
    // sits over the user's screen and will not go away.
    let focused = main
        .as_ref()
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(true);

    if !overlay_visible(recording, minimized, focused) {
        let _ = overlay.hide();
        return;
    }

    // A dialog this application opened is not another application. Both make
    // the main window lose focus, and only one of them is a reason to float a
    // card over the screen — over the file chooser Vesper itself just opened,
    // in fact, where its sliver can be hovered and expanded on top of it. The
    // window tells the backend when it opens one.
    if state.modal_open.load(Ordering::SeqCst) {
        let _ = overlay.hide();
        return;
    }

    let position = OverlayPosition::from_id(&state.settings.lock().overlay_position);
    let (collapsed, expanded_size) = footprints(position);
    let size = if expanded { expanded_size } else { collapsed };
    // The monitor the main window is on, not the primary: on a two-screen desk
    // the card belongs beside the work, and the scale factor differs per display.
    let monitor = main
        .as_ref()
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| overlay.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let pos = m.position();
        let msize = m.size();
        let (x, y) = dock_at(
            position,
            (pos.x, pos.y),
            (msize.width, msize.height),
            m.scale_factor(),
            size,
        );
        let _ = overlay.set_size(tauri::LogicalSize::new(size.0, size.1));
        let _ = overlay.set_position(tauri::PhysicalPosition::new(x, y));
    }
    // Sent on every sync, not read once at mount: changing the dock while the
    // app is running would otherwise leave the card drawing the previous edge —
    // and animating out of it — until the next launch.
    let _ = overlay.emit("overlay://position", position.id());
    let _ = overlay.show();
}

/// Tell the backend a native dialog this application opened is on screen.
///
/// The card hides while one is: a file chooser and another application look the
/// same from here — the main window loses focus either way — and floating an
/// always-on-top card over Vesper's own chooser is not what the setting asked
/// for.
#[tauri::command]
pub fn set_modal_open(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    open: bool,
) -> Result<(), String> {
    state.modal_open.store(open, Ordering::SeqCst);
    sync_overlay(&app, &state, false);
    Ok(())
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
        // Carried, not rebuilt: a client call has Requirements and Risks that
        // no field above holds, and dropping them here would erase them from
        // the meeting the moment anything else was improved.
        sections: m.sections.clone(),
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
    // From the segments, and from the stored ones when nothing is cached — which
    // is every meeting opened after a restart. Reading the flattened column there
    // gave up the timestamps this branch exists for, and gave the model whatever
    // names that column was written under rather than the ones in use now.
    let names = speaker_names(&state, &meeting);
    let segments = segments_of(&state, &id);
    let transcript = if segments.segments().is_empty() {
        meeting.transcript_text.clone()
    } else {
        segments.timestamped_text(&names)
    };
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
    // The sections carry a second copy of the field just improved. Without this
    // the version, the row and the export would all keep the body from before
    // the improvement — the stored copy is the one the screen reads.
    insights.sync_sections();
    let version = state
        .db
        .push_summary_version(&id, section.as_str(), &insights)?;
    state.db.save_insights(&id, &insights)?;
    meeting.summary = Some(insights.summary.clone());
    meeting.key_points = Some(insights.key_points_text());
    meeting.sections = insights.sections.clone();
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
        // The version's own sections, not the meeting's current ones: a restore
        // that kept today's Risks beside a summary from last week would be a set
        // that never existed.
        sections: wanted
            .sections_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default(),
    };
    let created = state.db.push_summary_version(&id, "restore", &insights)?;
    state.db.save_insights(&id, &insights)?;
    meeting.summary = Some(insights.summary.clone());
    meeting.key_points = Some(insights.key_points_text());
    meeting.sections = insights.sections.clone();
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

/// Name one of this meeting's two channels.
///
/// One channel and not the pair, for the reason an action item is written one
/// at a time: a caller that sends both sends its idea of the OTHER one too, and
/// that idea is stale the moment anything else writes. Renaming the second
/// channel while the first is still in flight would carry the first one's old
/// value back and erase a rename that had already succeeded.
///
/// Blank clears rather than stores: an empty string is not what anybody calls a
/// person, and clearing has to put the channel back to the app's own words in
/// whatever language the user reads — which is what `None` means in the row.
///
/// `transcript_text` is rebuilt here from the segments the meeting already has.
/// The prompts render their own copy — `prompt_transcript` — but this one is
/// what the search index is built from, and leaving it holding the old name
/// would have search answering with a name the meeting no longer uses.
/// `upsert_meeting` re-indexes in the same transaction.
#[tauri::command]
pub async fn set_speaker_name(
    state: State<'_, Arc<AppState>>,
    id: String,
    speaker: String,
    name: Option<String>,
) -> Result<MeetingRecord, String> {
    // From the WebView, so it is parsed rather than trusted: an unrecognised
    // channel has no right answer, and defaulting to one of the two would
    // rename whichever the caller did not mean.
    let speaker = Speaker::parse(&speaker).ok_or_else(|| "unknown speaker".to_string())?;
    // Refused while this is the meeting being recorded, for the reason an edit
    // to a line is: the transcription ticker writes the whole set back, and
    // `transcript_text` is rebuilt below from segments the next chunk is about
    // to replace.
    let recording_this = state
        .active_meeting
        .lock()
        .as_deref()
        .is_some_and(|active| active == id)
        && state.recorder.is_recording();
    if recording_this {
        return Err("the speakers can be renamed once the recording has stopped".into());
    }
    // The same lock the final pass takes, and taken for the same reason the
    // segment edit takes it: the recorder stopping is not enough. `stop_recording`
    // is still awaiting the last chunk holding a copy of this row read before
    // the rename, and it writes that copy back — so a name set in that window
    // would vanish with no sign it had been set.
    let _stt = state.stt_flight.lock().await;
    // And the same for the model calls, which hold a row for as long as the
    // provider takes to answer: a summary or a refinement started before the
    // rename would put its pre-rename copy of these two columns back. Taken in
    // the order `stop_recording` takes them, which is the only path that holds
    // both — the other order between two holders is what a deadlock is made of.
    let _refine = state.refine_flight.lock().await;
    let mut meeting = state
        .db
        .get_meeting(&id)?
        .ok_or_else(|| "meeting not found".to_string())?;
    // The WebView is the trust boundary and this name reaches a model prompt and
    // an exported document. Cleaned on the way in so the row holds exactly what
    // the window will show.
    let name = name.as_deref().and_then(clean_speaker_name);
    match speaker {
        Speaker::Me => meeting.speaker_me = name,
        Speaker::Others => meeting.speaker_others = name,
    }
    let transcript = state.db.load_transcript(&id)?;
    // Only when there is something to rebuild from. `save_transcript` clears the
    // segments before it writes them, so a meeting interrupted mid-write has
    // none and its `transcript_text` is the last copy of the words that exists.
    if !transcript.segments().is_empty() {
        meeting.transcript_text = transcript.plain_text(&speaker_names(&state, &meeting));
    }
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
