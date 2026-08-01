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
  llm_provider: "local" | "openrouter";
  openrouter_api_key?: string | null;
  openrouter_stt_model: string;
  openrouter_llm_model: string;
  local_stt_model: string;
  local_llm_model: string;
  reasoning_enabled: boolean;
  auto_summarize: boolean;
  language: string;
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

export interface ChatMessage {
  role: string;
  content: string;
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
  path: string;
  download_url?: string | null;
}

export const api = {
  listMeetings: () => invoke<MeetingRecord[]>("list_meetings"),
  getMeeting: (id: string) => invoke<MeetingRecord | null>("get_meeting", { id }),
  getTranscript: (id: string) => invoke<LiveTranscript>("get_transcript", { id }),
  deleteMeeting: (id: string) => invoke<void>("delete_meeting", { id }),
  search: (query: string) => invoke<SearchHit[]>("search_meetings_cmd", { query }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<AppSettings>("save_settings", { settings }),
  switchStt: (provider: string) => invoke<AppSettings>("switch_stt_provider", { provider }),
  switchLlm: (provider: string) => invoke<AppSettings>("switch_llm_provider", { provider }),
  setReasoning: (enabled: boolean) => invoke<AppSettings>("set_reasoning", { enabled }),
  recorderStatus: () => invoke<RecorderStatus>("recorder_status"),
  startRecording: (title?: string) => invoke<MeetingRecord>("start_recording", { title }),
  pauseRecording: () => invoke<RecorderStatus>("pause_recording"),
  resumeRecording: () => invoke<RecorderStatus>("resume_recording"),
  stopRecording: () => invoke<MeetingRecord>("stop_recording"),
  pollLiveStt: () => invoke<LiveTranscript>("poll_live_stt"),
  summarize: (id: string, template?: string) =>
    invoke<MeetingInsights>("summarize_meeting", { id, template }),
  chat: (id: string, question: string) => invoke<ChatMessage>("chat_meeting", { id, question }),
  listChat: (id: string) => invoke<ChatMessage[]>("list_chat", { id }),
  importAudio: (path: string, title?: string) =>
    invoke<MeetingRecord>("import_audio", { path, title }),
  retranscribe: (id: string) => invoke<MeetingRecord>("retranscribe", { id }),
  exportMeeting: (id: string, path: string, format: string) =>
    invoke<string>("export_meeting_cmd", { id, path, format }),
  listModels: () => invoke<ModelInfo[]>("list_models_cmd"),
  downloadModel: (modelId: string, url?: string) =>
    invoke<string>("download_model_cmd", { modelId, url }),
  updatesConfig: () => invoke<Record<string, unknown>>("check_updates_config"),
};

export function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`;
}

export function formatTs(ms: number): string {
  return `[${formatDuration(ms)}]`;
}
