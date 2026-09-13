//! Dictation: a take of the microphone that keeps every sentence as it is
//! transcribed, and types the result into the application that had the
//! keyboard when it started.
//!
//! The rules — which state follows which, whether a take may start, whether a
//! window is still the one the text was aimed at — live in `domain::dictation`.
//! This is the half with a microphone, a transcriber and other applications in
//! it.

mod encode;
mod platform;
mod verdict;

pub use platform::{can_insert, shortcut_refusal};

use crate::audio::capture::{read_dual_wav, repair_wav_header, DualChannelRecorder};
use crate::commands::{local_stt_ready, prompt_tail, AppState};
use crate::domain::dictation::{
    after_transcription, can_start, indicator_visible, next, recovered, still_the_target, Aim,
    DictationEvent, DictationState, FailureReason, InsertionOutcome, PressGate, Target, CAPTURE,
    INDICATOR_COLLAPSED, INDICATOR_EXPANDED,
};
use crate::domain::i18n::{t, Locale};
use crate::domain::overlay::{dock_at, OverlayPosition};
use crate::domain::segmenter::{Segmenter, Utterance};
use crate::domain::settings::AppSettings;
use crate::domain::shortcut::{dictation_status_from, ShortcutStatus, DICTATION_ACCELERATOR};
use crate::domain::speaker::Speaker;
use crate::paths::dictation_audio_dir;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;
use verdict::TypingClaim;

/// How often a take is drained and transcribed while it is being spoken — the
/// meeting's cadence, for the meeting's reason: a sentence is kept seconds after
/// it is said, not when the take ends.
const LIVE_INTERVAL_MS: u64 = 1_200;

/// How often the take is written to disk, so a crash costs at most this much.
const FLUSH_INTERVAL_MS: u64 = 1_200;

/// How long a finished take's result stays on the indicator.
const RESULT_HOLD_MS: u64 = 4_000;

/// How long a retried insertion waits for the user to put the cursor where the
/// text should go. The retry is pressed in Vesper, which has the keyboard.
pub const RETRY_COUNTDOWN_MS: u64 = 3_000;

/// How long an insertion may take to begin typing before it is given up on.
const INSERT_TIMEOUT: Duration = Duration::from_secs(8);

const STATE_EVENT: &str = "dictation://state";

/// Where dictation is, for the indicator and the main window.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DictationStatus {
    pub id: Option<String>,
    pub state: DictationState,
    /// Why the last attempt did not go as meant — a refused start or a failure.
    /// An i18n key, never a message from a platform API.
    pub reason_key: Option<String>,
    /// Set while a retried insertion counts down.
    pub countdown_ms: Option<u64>,
}

impl DictationStatus {
    fn idle() -> Self {
        Self {
            id: None,
            state: DictationState::Idle,
            reason_key: None,
            countdown_ms: None,
        }
    }

    fn of(id: &str, state: DictationState) -> Self {
        Self {
            id: Some(id.to_string()),
            ..Self::idle()
        }
        .with_state(state)
    }

    fn with_state(mut self, state: DictationState) -> Self {
        self.state = state;
        self
    }
}

/// Where a start or stop came from, which decides where the text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// The global shortcut: the keyboard is in the application the words are for.
    Shortcut,
    /// The indicator, which never takes focus, so the keyboard is still there.
    Indicator,
    /// Vesper's own window or its tray menu: the words are kept, not typed.
    Window,
}

struct Session {
    id: String,
    state: DictationState,
    aim: Aim,
    /// Where the take is cut into sentences. Only a listening take uses it.
    segmenter: Segmenter,
}

/// The dictation in progress, and what the indicator and the shortcut show.
///
/// Its own recorder, apart from the meeting's: dictation records the microphone
/// alone, and the two never run at once.
pub struct DictationService {
    recorder: DualChannelRecorder,
    /// Listening, transcribing or typing — a retry included. One at a time.
    session: Mutex<Option<Session>>,
    /// Retires a take's live pass and flush. Bumped by every start and stop.
    generation: AtomicU64,
    status: Mutex<DictationStatus>,
    /// Counts status changes, so a result's hold only clears the result it was
    /// for and never a take that started in the meantime.
    shown: AtomicU64,
    /// The pointer is on the indicator.
    expanded: AtomicBool,
    shortcut: Mutex<ShortcutStatus>,
    press: Mutex<PressGate>,
}

impl Default for DictationService {
    fn default() -> Self {
        Self::new()
    }
}

impl DictationService {
    pub fn new() -> Self {
        Self {
            recorder: DualChannelRecorder::new(),
            session: Mutex::new(None),
            generation: AtomicU64::new(0),
            status: Mutex::new(DictationStatus::idle()),
            shown: AtomicU64::new(0),
            expanded: AtomicBool::new(false),
            // Replaced during setup with what the OS answered.
            shortcut: Mutex::new(dictation_status_from(
                Err("not registered"),
                DICTATION_ACCELERATOR,
            )),
            press: Mutex::new(PressGate::default()),
        }
    }

    /// Listening, transcribing or typing, a retry included.
    pub fn is_active(&self) -> bool {
        self.session.lock().is_some()
    }

    /// Whether `id` is the dictation in progress, which may not be deleted from
    /// under itself.
    pub fn holds(&self, id: &str) -> bool {
        self.session.lock().as_ref().is_some_and(|s| s.id == id)
    }

    pub fn status(&self) -> DictationStatus {
        self.status.lock().clone()
    }

    pub fn shortcut_status(&self) -> ShortcutStatus {
        self.shortcut.lock().clone()
    }

    pub fn set_shortcut_status(&self, status: ShortcutStatus) {
        *self.shortcut.lock() = status;
    }

    /// A press of the shortcut — `false` for a combination held down and
    /// repeating.
    pub fn accept_press(&self) -> bool {
        self.press.lock().press(Instant::now())
    }

    pub fn release_press(&self) {
        self.press.lock().release();
    }

    fn advance(&self, id: &str, to: DictationState) {
        if let Some(s) = self.session.lock().as_mut().filter(|s| s.id == id) {
            s.state = to;
        }
    }

    /// Claims the session for a retry of `id`.
    fn claim(&self, id: &str, state: DictationState) -> Result<(), String> {
        let mut session = self.session.lock();
        if session.is_some() {
            return Err("dictation.gate.active".into());
        }
        *session = Some(Session {
            id: id.to_string(),
            state,
            aim: Aim::Keep,
            segmenter: Segmenter::new(16_000),
        });
        Ok(())
    }

    /// Lets go of the session, if it is still `id`'s.
    fn release(&self, id: &str) {
        let mut session = self.session.lock();
        if session.as_ref().is_some_and(|s| s.id == id) {
            *session = None;
        }
    }
}

/// Starts a dictation, or stops the one listening.
///
/// On the async runtime rather than the calling thread: the shortcut's callback
/// is the event loop, and a start can wait on the database.
pub fn toggle(app: &AppHandle, route: Route) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<Arc<AppState>>().inner().clone();
        let current = state.dictation.session.lock().as_ref().map(|s| s.state);
        match current {
            Some(DictationState::Listening) => stop(&app, &state).await,
            // Transcribing or typing. A press changes nothing until that is
            // done — acting on it could only be a second insertion.
            Some(_) => {}
            None => {
                // First, before anything is shown: where the keyboard is right
                // now is what the words are for.
                let aim = match route {
                    Route::Window => Aim::Keep,
                    Route::Shortcut | Route::Indicator => {
                        match tokio::task::spawn_blocking(platform::capture_target).await {
                            Ok(Some(target)) => Aim::At(target),
                            _ => Aim::Nowhere,
                        }
                    }
                };
                let handle = app.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let state = handle.state::<Arc<AppState>>();
                    start(&handle, &state, aim);
                })
                .await;
            }
        }
    });
}

fn meeting_active(state: &AppState) -> bool {
    state.recorder.is_recording() || state.active_meeting.lock().is_some()
}

fn start(app: &AppHandle, state: &AppState, aim: Aim) {
    let settings = state.settings.lock().clone();
    let gate = can_start(
        &settings,
        meeting_active(state),
        state.dictation.is_active(),
        local_stt_ready(&settings),
    );
    if !gate.allowed {
        refuse(
            app,
            state,
            gate.reason_key
                .as_deref()
                .unwrap_or("dictation.start_failed"),
        );
        return;
    }
    let id = Uuid::new_v4().to_string();
    {
        let mut session = state.dictation.session.lock();
        if session.is_some() {
            drop(session);
            refuse(app, state, "dictation.gate.active");
            return;
        }
        *session = Some(Session {
            id: id.clone(),
            state: DictationState::Listening,
            aim,
            segmenter: Segmenter::new(16_000),
        });
    }
    // Again, with the claim held. A meeting that began after the gate either
    // sees this session and refuses, or is seen here.
    if meeting_active(state) {
        state.dictation.release(&id);
        refuse(app, state, "dictation.gate.meeting");
        return;
    }
    let audio = dictation_audio_dir().join(format!("{id}.wav"));
    // The row first, naming the file, and only then the microphone: a take that
    // dies in its first second leaves a row recovery can find.
    let opened = std::fs::create_dir_all(dictation_audio_dir())
        .map_err(|e| e.to_string())
        .and_then(|()| state.db.create_dictation(&id))
        .and_then(|()| state.db.set_dictation_audio(&id, Some(&audio), 0))
        .and_then(|()| {
            state
                .dictation
                .recorder
                .start(audio.clone(), CAPTURE, settings.mic_device_id.clone(), None)
                .map_err(|e| e.to_string())
        });
    if opened.is_err() {
        // Nothing was captured, so nothing of it stays: no row, and no file.
        let _ = state.db.delete_dictation(&id);
        let _ = std::fs::remove_file(&audio);
        state.dictation.release(&id);
        tracing::warn!("a dictation could not open the microphone");
        refuse(app, state, "dictation.start_failed");
        return;
    }
    let generation = state.dictation.generation.fetch_add(1, Ordering::SeqCst) + 1;
    spawn_live_pass(app, id.clone(), generation);
    spawn_flush(app, generation);
    publish(
        app,
        state,
        DictationStatus::of(&id, DictationState::Listening),
    );
}

/// A start that did not happen, said where the user is looking: on the
/// indicator, and in a notification, since they may be in another application.
fn refuse(app: &AppHandle, state: &AppState, key: &str) {
    let mark = publish(
        app,
        state,
        DictationStatus {
            reason_key: Some(key.to_string()),
            ..DictationStatus::idle()
        },
    );
    // A second press while one is running is answered by the indicator alone.
    if key != "dictation.gate.active" {
        notify(app, state, &[key]);
    }
    hold(app, mark);
}

/// A system notification. It says why, and nothing of what was said.
fn notify(app: &AppHandle, state: &AppState, keys: &[&str]) {
    let locale = Locale::from_code(&state.settings.lock().ui_locale);
    let body = keys
        .iter()
        .map(|key| t(locale, key))
        .collect::<Vec<_>>()
        .join(" ");
    let _ = app
        .notification()
        .builder()
        .title(app.package_info().name.clone())
        .body(body)
        .show();
}

/// Stores a status and tells every window. Returns its place in the count, for
/// `hold`.
fn publish(app: &AppHandle, state: &AppState, status: DictationStatus) -> u64 {
    let mark = {
        // Stored, counted and sent under one lock, so two changes landing
        // together reach the windows in the order they were stored. Sending
        // does not wait on the event loop, so the lock cannot hold it up.
        let mut current = state.dictation.status.lock();
        *current = status;
        let mark = state.dictation.shown.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = app.emit(STATE_EVENT, &*current);
        mark
    };
    sync_indicator(app, state);
    mark
}

/// Takes a result off the indicator after a moment, unless something newer has
/// replaced it.
fn hold(app: &AppHandle, mark: u64) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(RESULT_HOLD_MS)).await;
        let state = app.state::<Arc<AppState>>();
        {
            let mut current = state.dictation.status.lock();
            if state.dictation.shown.load(Ordering::SeqCst) != mark {
                return;
            }
            *current = DictationStatus::idle();
            state.dictation.shown.fetch_add(1, Ordering::SeqCst);
            let _ = app.emit(STATE_EVENT, &*current);
        }
        sync_indicator(&app, &state);
    });
}

/// Shows, hides, sizes and places the indicator for what dictation is doing.
///
/// On the primary monitor's right edge, centred: dictation belongs to no window,
/// so there is no window whose monitor to follow.
pub fn sync_indicator(app: &AppHandle, state: &AppState) {
    let Some(window) = app.get_webview_window("dictation") else {
        return;
    };
    let status = state.dictation.status();
    let visible = indicator_visible(
        platform::floats_indicator(),
        state.settings.lock().dictation_indicator,
        state.recorder.is_recording(),
        status.state != DictationState::Idle || status.reason_key.is_some(),
    );
    if !visible {
        state.dictation.expanded.store(false, Ordering::SeqCst);
        let _ = window.hide();
        return;
    }
    let size = if state.dictation.expanded.load(Ordering::SeqCst) {
        INDICATOR_EXPANDED
    } else {
        INDICATOR_COLLAPSED
    };
    if let Some(monitor) = window.primary_monitor().ok().flatten() {
        let (x, y) = dock_at(
            OverlayPosition::RightCenter,
            (monitor.position().x, monitor.position().y),
            (monitor.size().width, monitor.size().height),
            monitor.scale_factor(),
            size,
        );
        let _ = window.set_size(tauri::LogicalSize::new(size.0, size.1));
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = window.show();
}

/// The indicator asking to open under the pointer, or to close again.
pub fn set_indicator_expanded(app: &AppHandle, state: &AppState, expanded: bool) {
    state.dictation.expanded.store(expanded, Ordering::SeqCst);
    sync_indicator(app, state);
}

fn spawn_live_pass(app: &AppHandle, id: String, generation: u64) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(LIVE_INTERVAL_MS)).await;
            if state.dictation.generation.load(Ordering::SeqCst) != generation
                || !state.dictation.recorder.is_recording()
            {
                break;
            }
            live_pass(&state, &id, generation).await;
        }
    });
}

/// Transcribes the sentences finished since the last pass, and keeps each.
///
/// Single-flight on the transcriber the meetings use: a pass still running when
/// the next tick comes is left to finish, and the audio waits in the recorder.
async fn live_pass(state: &AppState, id: &str, generation: u64) {
    let Ok(_flight) = state.stt_flight.try_lock() else {
        return;
    };
    // Past the lock. A stop that landed meanwhile has already taken the tail,
    // and whatever this drained now would be read twice.
    if state.dictation.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    let (mic, _, sample_rate) = state.dictation.recorder.drain_chunks();
    if mic.is_empty() {
        return;
    }
    let utterances = {
        let mut session = state.dictation.session.lock();
        match session.as_mut().filter(|s| s.id == id) {
            Some(s) => {
                s.segmenter.set_sample_rate(sample_rate);
                s.segmenter.push(&mic)
            }
            None => {
                state.dictation.recorder.rewind_chunks(mic.len(), 0);
                return;
            }
        }
    };
    if utterances.is_empty() {
        return;
    }
    let settings = state.settings.lock().clone();
    let untried = transcribe(state, &settings, id, utterances, sample_rate, 0).await;
    // Back into the segmenter, oldest at the head, for the next pass or the stop.
    if let Some(s) = state
        .dictation
        .session
        .lock()
        .as_mut()
        .filter(|s| s.id == id)
    {
        for u in untried.into_iter().rev() {
            s.segmenter.put_back(u);
        }
    }
}

/// Writes the take to disk while it lasts, on a thread of its own for the reason
/// a meeting's flush has one: a transcription that takes four seconds must not
/// make this a four-second tick.
fn spawn_flush(app: &AppHandle, generation: u64) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let _ = std::thread::Builder::new()
        .name("vesper-dictation-flush".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(FLUSH_INTERVAL_MS));
            if state.dictation.generation.load(Ordering::SeqCst) != generation
                || !state.dictation.recorder.is_recording()
            {
                break;
            }
            state.dictation.recorder.flush_to_disk();
        });
}

/// Transcribes utterances oldest first, keeping each sentence before the next is
/// tried. Returns what was not kept: the utterance that failed and everything
/// behind it.
///
/// Stopping at the first failure is what keeps the kept sentences contiguous,
/// and a retry resumes from the end of the last one.
async fn transcribe(
    state: &AppState,
    settings: &AppSettings,
    id: &str,
    utterances: Vec<Utterance>,
    sample_rate: u32,
    offset_ms: u64,
) -> Vec<Utterance> {
    let mut prompt = match state.db.dictation(id) {
        Ok(Some(d)) => prompt_tail(&d.text),
        _ => String::new(),
    };
    let mut left = utterances.into_iter();
    for u in left.by_ref() {
        let kept = match state
            .stt
            .transcribe_channel(
                settings,
                Speaker::Me,
                &u.pcm,
                sample_rate,
                u.start_ms + offset_ms,
                &prompt,
            )
            .await
        {
            Ok(chunk) => {
                let text = chunk.text.trim();
                if text.is_empty() {
                    true
                } else if state
                    .db
                    .add_dictation_segment(id, text, chunk.start_ms as i64, chunk.end_ms as i64)
                    .is_ok()
                {
                    prompt = prompt_tail(text);
                    true
                } else {
                    // Not stored is not kept, whatever the transcriber said.
                    false
                }
            }
            Err(_) => false,
        };
        if !kept {
            let mut untried = vec![u];
            untried.extend(left);
            return untried;
        }
    }
    Vec::new()
}

async fn stop(app: &AppHandle, state: &AppState) {
    let (id, aim) = {
        let mut session = state.dictation.session.lock();
        let Some(s) = session.as_mut() else {
            return;
        };
        // A stop that arrives second finds the take already transcribing.
        let Ok(to) = next(s.state, DictationEvent::Stop) else {
            return;
        };
        s.state = to;
        (s.id.clone(), s.aim)
    };
    let saved = state.dictation.recorder.stop();
    // Retires the live pass and the flush with the take they belong to.
    state.dictation.generation.fetch_add(1, Ordering::SeqCst);
    let duration_ms = state.dictation.recorder.elapsed_ms() as i64;
    // Taken now, before any await, while nothing else has touched the recorder.
    let (tail, _, sample_rate) = state.dictation.recorder.drain_chunks();
    match &saved {
        Ok(path) => {
            let _ = state.db.set_dictation_audio(&id, Some(path), duration_ms);
        }
        // The row still names the file, whatever of it was written.
        Err(_) => tracing::warn!("a dictation's audio could not be finished"),
    }
    let _ = state
        .db
        .set_dictation_state(&id, DictationState::Transcribing, None);
    publish(
        app,
        state,
        DictationStatus::of(&id, DictationState::Transcribing),
    );

    // A live pass still transcribing holds this, and puts back what it could
    // not finish before letting go.
    let flight = state.stt_flight.lock().await;
    let utterances = {
        let mut session = state.dictation.session.lock();
        match session.as_mut().filter(|s| s.id == id) {
            Some(s) => {
                let mut segmenter =
                    std::mem::replace(&mut s.segmenter, Segmenter::new(sample_rate));
                segmenter.set_sample_rate(sample_rate);
                let mut all = segmenter.push(&tail);
                all.extend(segmenter.flush());
                all
            }
            None => Vec::new(),
        }
    };
    let settings = state.settings.lock().clone();
    let untried = transcribe(state, &settings, &id, utterances, sample_rate, 0).await;
    drop(flight);
    if !untried.is_empty() {
        // What landed is kept, and so is the audio, for a retry to finish from.
        let to = next(
            DictationState::Transcribing,
            DictationEvent::TranscriptionFailed,
        )
        .unwrap_or(DictationState::TranscriptionFailed);
        conclude(app, state, &id, to, None);
        return;
    }
    // Transcribed, so the take is not history any more.
    if state.db.forget_dictation_audio(&id, duration_ms).is_err() {
        tracing::warn!("a transcribed dictation's audio could not be deleted");
    }
    let text = match state.db.dictation(&id) {
        Ok(Some(d)) => d.text,
        _ => String::new(),
    };
    let decided = after_transcription(
        !text.is_empty(),
        platform::can_insert(),
        aim,
        std::process::id(),
    );
    let transcribed = DictationEvent::Transcribed {
        has_text: !text.is_empty(),
        will_insert: decided.is_ok(),
    };
    let target = match (next(DictationState::Transcribing, transcribed), decided) {
        (Ok(DictationState::Inserting), Ok(target)) => target,
        (_, decided) => {
            conclude(
                app,
                state,
                &id,
                DictationState::Saved,
                decided.err().flatten(),
            );
            return;
        }
    };
    state.dictation.advance(&id, DictationState::Inserting);
    let _ = state
        .db
        .set_dictation_state(&id, DictationState::Inserting, None);
    publish(
        app,
        state,
        DictationStatus::of(&id, DictationState::Inserting),
    );
    let (to, reason) = settled(insert_into(Some(target), text).await);
    conclude(app, state, &id, to, reason);
}

/// Types `text` where the keyboard is — if that is still `aimed`, or, for a
/// retry the user pointed by hand, anywhere but Vesper.
async fn insert_into(aimed: Option<Target>, text: String) -> InsertionOutcome {
    let claim = Arc::new(TypingClaim::default());
    let typing = Arc::clone(&claim);
    let own = std::process::id();
    let mut task = tokio::task::spawn_blocking(move || {
        // A shortcut's keys are still down when the transcript is this quick,
        // and text typed under Ctrl or Alt is a stream of shortcuts.
        if !platform::wait_for_modifiers() {
            return InsertionOutcome::Failed(FailureReason::Timeout);
        }
        // Asked again after that wait, which was time the user had to move.
        let now = platform::capture_target();
        let target = match aimed {
            Some(aimed) => match still_the_target(aimed, now, own) {
                Ok(()) => aimed,
                Err(reason) => return InsertionOutcome::Failed(reason),
            },
            None => match now {
                Some(now) if now.process != own => now,
                _ => return InsertionOutcome::Failed(FailureReason::NoEditableTarget),
            },
        };
        platform::insert(target, &text, &typing)
    });
    match tokio::time::timeout(INSERT_TIMEOUT, &mut task).await {
        Ok(joined) => joined.unwrap_or(InsertionOutcome::Failed(FailureReason::Failed)),
        // Not begun, and now it never will.
        Err(_) if claim.give_up() => InsertionOutcome::Failed(FailureReason::Timeout),
        // Already typing: what it concludes is the answer.
        Err(_) => task
            .await
            .unwrap_or(InsertionOutcome::Failed(FailureReason::Failed)),
    }
}

fn settled(outcome: InsertionOutcome) -> (DictationState, Option<FailureReason>) {
    let to = next(
        DictationState::Inserting,
        DictationEvent::Insertion(outcome),
    )
    .unwrap_or(DictationState::InsertionFailed);
    let reason = match outcome {
        InsertionOutcome::Failed(reason) => Some(reason),
        _ => None,
    };
    (to, reason)
}

/// Records how a take or a retry ended, lets go of the session and says so.
fn conclude(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    to: DictationState,
    reason: Option<FailureReason>,
) {
    if state.db.set_dictation_state(id, to, reason).is_err() {
        tracing::warn!("a dictation's outcome could not be stored");
    }
    // Before anyone is told, so a window that reacts by asking what it may start
    // gets the true answer.
    state.dictation.release(id);
    let mark = publish(
        app,
        state,
        DictationStatus {
            reason_key: reason.map(|r| r.key().to_string()),
            ..DictationStatus::of(id, to)
        },
    );
    match (to, reason) {
        (DictationState::TranscriptionFailed, _) => notify(
            app,
            state,
            &[
                "dictation.notify.transcription_failed",
                "dictation.notify.kept",
            ],
        ),
        (DictationState::InsertionFailed | DictationState::Saved, Some(reason)) => {
            notify(app, state, &[reason.key(), "dictation.notify.kept"])
        }
        _ => {}
    }
    hold(app, mark);
}

/// Types a kept dictation again, after a countdown for the user to put the
/// cursor where it should go.
pub async fn retry_insertion(app: &AppHandle, state: &AppState, id: String) -> Result<(), String> {
    if !platform::can_insert() {
        return Err(FailureReason::Unsupported.key().into());
    }
    let retryable = |state: &AppState| -> Result<String, String> {
        match state.db.dictation(&id)? {
            Some(d)
                if !d.text.is_empty() && next(d.state, DictationEvent::RetryInsertion).is_ok() =>
            {
                Ok(d.text)
            }
            Some(_) => Err("dictation.retry.not_retryable".into()),
            None => Err("dictation not found".into()),
        }
    };
    retryable(state)?;
    state.dictation.claim(&id, DictationState::Inserting)?;
    // Read again with the claim held: a delete that landed in between is refused
    // from here on, and must not be typed out three seconds after it.
    let text = match retryable(state) {
        Ok(text) => text,
        Err(e) => {
            state.dictation.release(&id);
            return Err(e);
        }
    };
    let _ = state
        .db
        .set_dictation_state(&id, DictationState::Inserting, None);
    publish(
        app,
        state,
        DictationStatus {
            countdown_ms: Some(RETRY_COUNTDOWN_MS),
            ..DictationStatus::of(&id, DictationState::Inserting)
        },
    );
    tokio::time::sleep(Duration::from_millis(RETRY_COUNTDOWN_MS)).await;
    let (to, reason) = settled(insert_into(None, text).await);
    conclude(app, state, &id, to, reason);
    Ok(())
}

/// Transcribes the rest of a take whose transcription failed, from the end of
/// the last sentence kept.
pub async fn retry_transcription(
    app: &AppHandle,
    state: &AppState,
    id: String,
) -> Result<(), String> {
    let record = state
        .db
        .dictation(&id)?
        .ok_or_else(|| "dictation not found".to_string())?;
    if next(record.state, DictationEvent::RetryTranscription).is_err() {
        return Err("dictation.retry.not_retryable".into());
    }
    let Some(audio) = state.db.dictation_audio(&id)? else {
        return Err("dictation.retry.no_audio".into());
    };
    let settings = state.settings.lock().clone();
    // The transcriber's own refusals — a missing model, the cloud without
    // consent — and never beside a meeting, which is using it.
    let gate = can_start(
        &settings,
        meeting_active(state),
        state.dictation.is_active(),
        local_stt_ready(&settings),
    );
    if !gate.allowed {
        return Err(gate
            .reason_key
            .unwrap_or_else(|| "dictation.start_failed".into()));
    }
    state.dictation.claim(&id, DictationState::Transcribing)?;
    let _ = state
        .db
        .set_dictation_state(&id, DictationState::Transcribing, None);
    publish(
        app,
        state,
        DictationStatus::of(&id, DictationState::Transcribing),
    );

    let flight = state.stt_flight.lock().await;
    // A take cut short by a crash has a header that never learned its length.
    let _ = repair_wav_header(&audio);
    let finished = match (
        read_dual_wav(&audio),
        state.db.dictation_accepted_until(&id),
    ) {
        (Ok((mic, _, sample_rate)), Ok(accepted_ms)) => {
            let from_ms = u64::try_from(accepted_ms).unwrap_or(0);
            let from =
                usize::try_from(from_ms * u64::from(sample_rate) / 1000).unwrap_or(usize::MAX);
            let mut segmenter = Segmenter::new(sample_rate);
            let mut utterances = segmenter.push(mic.get(from..).unwrap_or_default());
            utterances.extend(segmenter.flush());
            transcribe(state, &settings, &id, utterances, sample_rate, from_ms)
                .await
                .is_empty()
        }
        _ => false,
    };
    drop(flight);
    if finished {
        if state
            .db
            .forget_dictation_audio(&id, record.duration_ms)
            .is_err()
        {
            tracing::warn!("a transcribed dictation's audio could not be deleted");
        }
        // Kept, not typed: the window it was aimed at is long gone.
        let transcribed = DictationEvent::Transcribed {
            has_text: true,
            will_insert: false,
        };
        let to = next(DictationState::Transcribing, transcribed).unwrap_or(DictationState::Saved);
        conclude(app, state, &id, to, None);
    } else {
        let to = next(
            DictationState::Transcribing,
            DictationEvent::TranscriptionFailed,
        )
        .unwrap_or(DictationState::TranscriptionFailed);
        conclude(app, state, &id, to, None);
    }
    Ok(())
}

/// Settles what a previous run left mid-take, before anything can start.
pub fn recover(state: &AppState) {
    let Ok(rows) = state.db.unfinished_dictations() else {
        return;
    };
    for row in rows {
        match recovered(row.state, row.has_audio, !row.text.is_empty()) {
            None => {
                let _ = state.db.delete_dictation(&row.id);
            }
            Some((to, reason)) => {
                let _ = state.db.set_dictation_state(&row.id, to, reason);
            }
        }
    }
}
