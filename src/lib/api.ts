import { invoke } from "@tauri-apps/api/core";

export type Speaker = "me" | "others";

export interface TranscriptSegment {
  id: string;
  speaker: Speaker;
  text: string;
  start_ms: number;
  end_ms: number;
}

export interface LiveTranscript {
  segments: TranscriptSegment[];
}

export interface MeetingRecord {
  /// Provider spend for this meeting, and the same figure formatted by the
  /// backend. Absent on a meeting that never called a paid provider, which is
  /// why the header renders nothing rather than a zero.
  cost_nano_usd?: number | null;
  cost_label?: string | null;
  id: string;
  title: string;
  status: string;
  created_at: string;
  updated_at: string;
  duration_ms: number;
  audio_path?: string | null;
  transcript_text: string;
  summary?: string | null;
  action_items?: string | null;
  key_points?: string | null;
  project?: string | null;
}

export interface AppSettings {
  stt_provider: "local" | "openrouter";
  llm_provider: "local" | "openrouter" | "openai_compatible";
  endpoint_base_url: string;
  endpoint_model: string;
  openrouter_api_key?: string | null;
  openrouter_stt_model: string;
  openrouter_llm_model: string;
  /** Ids the user picked before, most recent first, capped by the backend.
   *  Optional because the field lands in a separate change — every consumer
   *  renders without it and it lights up on its own the day it arrives. */
  recent_openrouter_llm_models?: string[];
  local_stt_model: string;
  local_llm_model: string;
  reasoning_enabled: boolean;
  auto_summarize: boolean;
  language: string;
  ui_locale: string;
  /** `light` | `dark`. Optional because the field lands with the backend change;
   *  absent reads as light, which is the product default — so the front end
   *  behaves correctly on its own until the backend starts keeping it. */
  theme?: string;
  onboarding_complete: boolean;
  mic_device_id?: string | null;
  system_device_id?: string | null;
  compute_backend: string;
  confirm_before_recording: boolean;
}

export interface ChannelLevels {
  me_peak: number;
  me_rms: number;
  others_peak: number;
  others_rms: number;
}

export interface RecorderStatus {
  recording: boolean;
  paused: boolean;
  meeting_id?: string | null;
  elapsed_ms: number;
  levels: ChannelLevels;
}

export interface SearchHit {
  meeting_id: string;
  title: string;
  snippet: string;
  score: number;
}

/** Where a meeting is between Stop and its summary.
 *
 *  No percentage, deliberately: transcription is one blocking pass over the
 *  whole buffer and the summary is one non-streamed request, so there is no
 *  progress to report and a number would have to be invented. */
export type MeetingPhase =
  | "saving"
  | "transcribing"
  | "summarizing"
  | "ready"
  | "summary_failed";

/** Payload of the `meeting://progress` event.
 *
 *  Optional by absence: the backend that emits it is a separate change, and
 *  until it lands the event never fires and nothing renders. These names are
 *  the entire contract between the two halves and nothing type-checks across
 *  the WebView boundary — they must match the Rust `#[serde(rename_all =
 *  "snake_case")]` variants exactly. */
export interface MeetingProgress {
  meeting_id: string;
  phase: MeetingPhase;
  error?: string | null;
}

export interface ChatMessage {
  role: string;
  content: string;
}

export interface ContextNote {
  id: number;
  text: string;
  /// Offset from the start of the recording. Null for a note written after it
  /// stopped, which has no position to hold against the transcript.
  at_ms: number | null;
  created_at: string;
}

export interface ActionItem {
  id: number;
  text: string;
  owner: string | null;
  due: string | null;
  status: "open" | "done";
  /// Who put it there. The next summary may replace what the model said and
  /// never what a person said.
  source: "ai" | "user";
  edited: boolean;
}

export interface MeetingInsights {
  summary: string;
  key_points: string[];
  action_items: string[];
}

export interface ModelInfo {
  id: string;
  kind: string;
  label: string;
  ready: boolean;
  /** File is on disk. Distinct from ready, which also requires verification. */
  present: boolean;
  path: string;
  download_url?: string | null;
  /** What the catalog says the finished artifact weighs. Already on the wire;
   *  it was simply never declared on this side. */
  size_hint_bytes?: number | null;
  /** Bytes of a `.part` file sitting beside the artifact — a download that was
   *  started and never finished. Optional because the backend that reports it
   *  is a separate change: until it lands the field is absent and the slot
   *  renders exactly as it does today. */
  partial_bytes?: number | null;
}

/** Payload of the `models://download-progress` event.
 *
 *  `bytes_per_sec`, `eta_secs`, `attempt` and `resumed_from_bytes` are optional
 *  because the backend that emits them is a separate change that has not landed
 *  yet. Every consumer must render without them; they light up on their own the
 *  day the Rust side starts sending them, with no front-end change. */
export interface DownloadProgress {
  model_id: string;
  downloaded_bytes: number;
  total_bytes?: number | null;
  done: boolean;
  error?: string | null;
  phase: string;
  bytes_per_sec?: number | null;
  eta_secs?: number | null;
  attempt?: number | null;
  resumed_from_bytes?: number | null;
}

export interface AudioDevice {
  id: string;
  name: string;
  kind: "mic" | "system";
  is_default: boolean;
}

export interface StartGate {
  allowed: boolean;
  reason?: string | null;
  reason_key?: string | null;
}

/** Whether the OS accepted the global accelerator, and what it is.
 *
 *  The command that reports this lands with the backend change, so
 *  `api.shortcutStatus()` rejects until then — every caller must tolerate a
 *  `null` and name the key without a note. The in-app listener makes the key
 *  work while the window is focused either way; only the note about the global
 *  registration waits on the backend. */
export interface ShortcutStatus {
  registered: boolean;
  accelerator: string;
  reason_key?: string | null;
}

export interface SummaryVersion {
  version: number;
  /// `summarize`, `key_points`, `action_items` or `restore`.
  origin: string;
  created_at: string;
  summary: string;
  key_points: string;
  action_items: string;
}

export interface OrModel {
  id: string;
  name: string;
  /// What the model charges per million tokens. Absent when the catalogue did
  /// not quote one — unknown, never free.
  price_label?: string | null;
  kind: string;
}

export interface CapabilityReport {
  cpu_cores: number;
  cuda_available: boolean;
  cuda_device_name?: string | null;
  vulkan_available: boolean;
  gpu_name?: string | null;
  vram_mb: number;
  recommended_stt_model: string;
  recommended_llm_model: string;
  recommended_backend: string;
  notes: string[];
}

export const api = {
  listMeetings: () => invoke<MeetingRecord[]>("list_meetings"),
  getMeeting: (id: string) => invoke<MeetingRecord | null>("get_meeting", { id }),
  getTranscript: (id: string) => invoke<LiveTranscript>("get_transcript", { id }),
  deleteMeeting: (id: string) => invoke<void>("delete_meeting", { id }),
  search: (query: string) => invoke<SearchHit[]>("search_meetings_cmd", { query }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<AppSettings>("save_settings", { settings }),
  /// The theme alone. A whole-settings write for one field is a lost update
  /// waiting to happen: a toggle still in flight lands after a drawer Save and
  /// carries the pre-drawer value of every other field.
  setTheme: (theme: string) => invoke<AppSettings>("set_theme", { theme }),
  completeOnboarding: (settings: AppSettings) =>
    invoke<AppSettings>("complete_onboarding", { settings }),
  switchStt: (provider: string) => invoke<AppSettings>("switch_stt_provider", { provider }),
  switchLlm: (provider: string) => invoke<AppSettings>("switch_llm_provider", { provider }),
  setReasoning: (enabled: boolean) => invoke<AppSettings>("set_reasoning", { enabled }),
  recorderStatus: () => invoke<RecorderStatus>("recorder_status"),
  canRecord: () => invoke<StartGate>("can_record"),
  shortcutStatus: () => invoke<ShortcutStatus>("shortcut_status"),
  listDevices: () => invoke<AudioDevice[]>("list_audio_devices_cmd"),
  i18nCatalog: (locale: string) =>
    invoke<Record<string, string>>("get_i18n_catalog", { locale }),
  capabilities: () => invoke<CapabilityReport>("get_capabilities"),
  openrouterSttModels: () => invoke<OrModel[]>("list_openrouter_stt_models"),
  openrouterLlmModels: () => invoke<OrModel[]>("list_openrouter_llm_models"),
  startRecording: (title?: string) => invoke<MeetingRecord>("start_recording", { title }),
  pauseRecording: () => invoke<RecorderStatus>("pause_recording"),
  resumeRecording: () => invoke<RecorderStatus>("resume_recording"),
  stopRecording: () => invoke<MeetingRecord>("stop_recording"),
  /// Every stored version of a meeting's insights, oldest first. Backfills a
  /// baseline for meetings summarised before versioning existed.
  listSummaryVersions: (id: string) =>
    invoke<SummaryVersion[]>("list_summary_versions", { id }),
  /// Improve one section. Rejects if the model does not answer with a list —
  /// the meeting keeps what it had rather than gaining a sentence about failure.
  refineSummarySection: (id: string, section: "key_points" | "action_items") =>
    invoke<SummaryVersion>("refine_summary_section", { id, section }),
  /// Puts an earlier version back, as a new version. Nothing is rewound.
  restoreSummaryVersion: (id: string, version: number) =>
    invoke<SummaryVersion>("restore_summary_version", { id, version }),
  /// The card asking to grow or shrink as the pointer arrives and leaves.
  /// A no-op unless a recording is running and the main window is minimized —
  /// the backend re-derives both rather than trusting a remembered flag.
  setOverlayExpanded: (expanded: boolean) =>
    invoke<void>("set_overlay_expanded", { expanded }),
  summarize: (id: string, template?: string) =>
    invoke<MeetingInsights>("summarize_meeting", { id, template }),
  importAudio: (path: string, title?: string) =>
    invoke<MeetingRecord>("import_audio", { path, title }),
  // The only new command on this side that cannot degrade to nothing: a rename
  // the backend has not learned yet rejects, and the caller surfaces that
  // rather than showing a title the database does not carry.
  renameMeeting: (id: string, title: string) =>
    invoke<MeetingRecord>("rename_meeting", { id, title }),
  // The title, sanitised for the filesystem by the side that owns the rule
  // table. Every caller must have a fallback name — it lands with the backend
  // change and rejects until then.
  suggestedExportName: (id: string, format: string) =>
    invoke<string>("suggested_export_name", { id, format }),
  exportMeeting: (id: string, path: string, format: string) =>
    invoke<string>("export_meeting_cmd", { id, path, format }),
  /// Ask a question about one meeting. The answer is the whole answer — this
  /// path does not stream, so the wait is silent and the caller owns saying so.
  chatMeeting: (id: string, question: string) =>
    invoke<ChatMessage>("chat_meeting", { id, question }),
  listChat: (id: string) => invoke<ChatMessage[]>("list_chat", { id }),
  /// The note comes back stamped by the backend: it knows where the recording
  /// is, and the window only knows what it painted.
  addContextNote: (id: string, text: string) =>
    invoke<ContextNote>("add_context_note", { id, text }),
  listContextNotes: (id: string) =>
    invoke<ContextNote[]>("list_context_notes", { id }),
  deleteContextNote: (id: string, noteId: number) =>
    invoke<void>("delete_context_note", { id, noteId }),
  listActionItems: (id: string) =>
    invoke<ActionItem[]>("list_action_items", { id }),
  /// One item at a time. Each returns the whole list as stored, because a
  /// summary may have merged in between — but none of them *sends* a list, so
  /// none can carry a stale idea of the items it did not touch.
  addActionItem: (id: string, text: string) =>
    invoke<ActionItem[]>("add_action_item", { id, text }),
  updateActionItem: (id: string, item: ActionItem) =>
    invoke<ActionItem[]>("update_action_item", { id, item }),
  deleteActionItem: (id: string, itemId: number) =>
    invoke<ActionItem[]>("delete_action_item", { id, itemId }),
  listModels: () => invoke<ModelInfo[]>("list_models_cmd"),
  // No URL parameter: the backend resolves it from its own catalog and verifies
  // the artifact's checksum before it reaches whisper.cpp / llama.cpp.
  downloadModel: (modelId: string) => invoke<string>("download_model_cmd", { modelId }),
  updatesConfig: () => invoke<Record<string, unknown>>("check_updates_config"),
};

/** Rolls the hour out of the minutes field once there is one. Without it a
 *  90-minute meeting read `90:00`, and the clock is the one number a meeting
 *  recorder must get right. */
export function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const r = s % 60;
  const rest = `${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`;
  return h > 0 ? `${h}:${rest}` : rest;
}
