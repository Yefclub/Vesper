import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { motion, AnimatePresence } from "framer-motion";
import {
  FileAudio,
  MessageSquare,
  Mic,
  Pause,
  Play,
  Search,
  Settings,
  Square,
  Sparkles,
} from "lucide-react";
import {
  api,
  AppSettings,
  ChatMessage,
  formatDuration,
  formatTs,
  LiveTranscript,
  MeetingRecord,
  ModelInfo,
  RecorderStatus,
  SearchHit,
} from "./lib/api";
import { LevelMeter } from "./components/LevelMeter";
import { Sidebar } from "./components/Sidebar";
import { SettingsPanel } from "./components/SettingsPanel";

type Tab = "transcript" | "summary" | "chat";

export default function App() {
  const [meetings, setMeetings] = useState<MeetingRecord[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<LiveTranscript>({ segments: [] });
  const [status, setStatus] = useState<RecorderStatus | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [tab, setTab] = useState<Tab>("transcript");
  const [chat, setChat] = useState<ChatMessage[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [updateNote, setUpdateNote] = useState<string | null>(null);

  const selected = useMemo(
    () => meetings.find((m) => m.id === selectedId) ?? null,
    [meetings, selectedId],
  );

  const refreshMeetings = useCallback(async () => {
    try {
      const list = await api.listMeetings();
      setMeetings(list);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadMeeting = useCallback(async (id: string) => {
    setSelectedId(id);
    try {
      const [t, c, m] = await Promise.all([
        api.getTranscript(id),
        api.listChat(id),
        api.getMeeting(id),
      ]);
      setTranscript(t);
      setChat(c);
      if (m) {
        setMeetings((prev) => {
          const others = prev.filter((x) => x.id !== id);
          return [m, ...others];
        });
      }
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    (async () => {
      try {
        await refreshMeetings();
        setSettings(await api.getSettings());
        setModels(await api.listModels());
        setStatus(await api.recorderStatus());
      } catch (e) {
        setError(String(e));
      }
      // Auto-update check (silent when no update / offline)
      try {
        const update = await check();
        if (update) {
          setUpdateNote(`Update ${update.version} available — downloading…`);
          await update.downloadAndInstall();
          setUpdateNote("Update installed. Relaunching…");
          await relaunch();
        }
      } catch {
        // No signed update server yet — ignore.
      }
    })();
  }, [refreshMeetings]);

  useEffect(() => {
    let unsubs: Array<() => void> = [];
    (async () => {
      unsubs.push(
        await listen<LiveTranscript>("transcript://append", (e) => {
          setTranscript(e.payload);
        }),
      );
      unsubs.push(
        await listen<MeetingRecord>("meeting://ready", (e) => {
          setMeetings((prev) => {
            const rest = prev.filter((m) => m.id !== e.payload.id);
            return [e.payload, ...rest];
          });
          setSelectedId(e.payload.id);
        }),
      );
      unsubs.push(
        await listen("hotkey://toggle-record", async () => {
          try {
            const st = await api.recorderStatus();
            if (st.recording) await handleStop();
            else await handleStart();
          } catch (e) {
            setError(String(e));
          }
        }),
      );
    })();
    return () => unsubs.forEach((u) => u());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Poll levels + live STT while recording
  useEffect(() => {
    if (!status?.recording) return;
    const id = window.setInterval(async () => {
      try {
        setStatus(await api.recorderStatus());
        const t = await api.pollLiveStt();
        setTranscript(t);
      } catch {
        /* keep UI alive */
      }
    }, 1200);
    return () => window.clearInterval(id);
  }, [status?.recording, status?.paused]);

  async function handleStart() {
    setBusy(true);
    setError(null);
    try {
      const m = await api.startRecording();
      setStatus(await api.recorderStatus());
      setSelectedId(m.id);
      setTranscript({ segments: [] });
      setChat([]);
      setTab("transcript");
      await refreshMeetings();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    try {
      const m = await api.stopRecording();
      setStatus(await api.recorderStatus());
      await refreshMeetings();
      await loadMeeting(m.id);
      setTab("summary");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handlePauseResume() {
    try {
      if (status?.paused) setStatus(await api.resumeRecording());
      else setStatus(await api.pauseRecording());
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleSearch(q: string) {
    setQuery(q);
    if (!q.trim()) {
      setHits([]);
      return;
    }
    try {
      setHits(await api.search(q));
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleChat() {
    if (!selectedId || !question.trim()) return;
    setBusy(true);
    try {
      const userMsg = { role: "user", content: question };
      setChat((c) => [...c, userMsg]);
      setQuestion("");
      const ans = await api.chat(selectedId, userMsg.content);
      setChat((c) => [...c, ans]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleSummarize(template = "general") {
    if (!selectedId) return;
    setBusy(true);
    try {
      await api.summarize(selectedId, template);
      await loadMeeting(selectedId);
      setTab("summary");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleImport() {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "Audio", extensions: ["wav", "mp3", "m4a", "webm"] }],
      });
      if (!file || Array.isArray(file)) return;
      setBusy(true);
      const m = await api.importAudio(file);
      await refreshMeetings();
      await loadMeeting(m.id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleExport(format: "md" | "pdf" | "docx") {
    if (!selectedId) return;
    try {
      const path = await save({
        defaultPath: `vesper-export.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!path) return;
      await api.exportMeeting(selectedId, path, format);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleDelete(id: string) {
    try {
      await api.deleteMeeting(id);
      if (selectedId === id) {
        setSelectedId(null);
        setTranscript({ segments: [] });
        setChat([]);
      }
      await refreshMeetings();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="flex h-full bg-background text-foreground">
      <Sidebar
        meetings={meetings}
        selectedId={selectedId}
        query={query}
        hits={hits}
        onSearch={handleSearch}
        onSelect={loadMeeting}
        onDelete={handleDelete}
        onOpenSettings={() => setShowSettings(true)}
        onImport={handleImport}
      />

      <main className="flex min-w-0 flex-1 flex-col">
        {/* Top bar / recorder */}
        <header className="flex items-center gap-4 border-b border-border bg-surface px-6 py-4">
          <div className="flex items-center gap-2">
            <div className="text-lg font-semibold tracking-tight">
              <span className="bg-gradient-to-r from-accent to-accent-2 bg-clip-text text-transparent">
                Vesper
              </span>
            </div>
            <span className="rounded-full bg-surface-3 px-2 py-0.5 text-[10px] uppercase tracking-wider text-muted">
              private
            </span>
          </div>

          <div className="mx-auto flex items-center gap-3">
            {!status?.recording ? (
              <button
                data-testid="btn-record"
                disabled={busy}
                onClick={handleStart}
                className="flex items-center gap-2 rounded-full bg-accent px-5 py-2 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-50"
              >
                <Mic size={16} /> Record
              </button>
            ) : (
              <>
                <button
                  data-testid="btn-pause"
                  onClick={handlePauseResume}
                  className="flex items-center gap-2 rounded-full bg-surface-3 px-4 py-2 text-sm text-foreground hover:bg-border"
                >
                  {status.paused ? <Play size={16} /> : <Pause size={16} />}
                  {status.paused ? "Resume" : "Pause"}
                </button>
                <button
                  data-testid="btn-stop"
                  onClick={handleStop}
                  className="flex items-center gap-2 rounded-full bg-danger/20 px-4 py-2 text-sm text-danger hover:bg-danger/30"
                >
                  <Square size={14} /> Stop
                </button>
                <span className="font-mono text-sm text-muted" data-testid="elapsed">
                  {formatDuration(status.elapsed_ms)}
                </span>
              </>
            )}
          </div>

          <div className="flex items-center gap-3">
            {status?.recording && (
              <LevelMeter levels={status.levels} />
            )}
            <button
              onClick={() => setShowSettings(true)}
              className="rounded-lg p-2 text-muted hover:bg-surface-3 hover:text-foreground"
              title="Settings"
            >
              <Settings size={18} />
            </button>
          </div>
        </header>

        {error && (
          <div className="border-b border-danger/30 bg-danger/10 px-6 py-2 text-sm text-danger">
            {error}
            <button className="ml-3 underline" onClick={() => setError(null)}>
              dismiss
            </button>
          </div>
        )}
        {updateNote && (
          <div className="border-b border-accent/30 bg-accent/10 px-6 py-2 text-sm text-accent">
            {updateNote}
          </div>
        )}

        {/* Content */}
        <div className="flex min-h-0 flex-1 flex-col">
          {selected ? (
            <>
              <div className="flex items-center justify-between border-b border-border px-6 py-3">
                <div>
                  <h1 className="text-base font-medium">{selected.title}</h1>
                  <p className="text-xs text-muted">
                    {selected.status} · {formatDuration(selected.duration_ms)}
                  </p>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => handleSummarize("general")}
                    className="flex items-center gap-1 rounded-lg bg-surface-3 px-3 py-1.5 text-xs hover:bg-border"
                  >
                    <Sparkles size={14} /> Summarize
                  </button>
                  <button
                    onClick={() => handleExport("md")}
                    className="rounded-lg bg-surface-3 px-3 py-1.5 text-xs hover:bg-border"
                  >
                    MD
                  </button>
                  <button
                    onClick={() => handleExport("pdf")}
                    className="rounded-lg bg-surface-3 px-3 py-1.5 text-xs hover:bg-border"
                  >
                    PDF
                  </button>
                  <button
                    onClick={() => handleExport("docx")}
                    className="rounded-lg bg-surface-3 px-3 py-1.5 text-xs hover:bg-border"
                  >
                    DOCX
                  </button>
                  <button
                    onClick={() => selectedId && api.retranscribe(selectedId).then((m) => loadMeeting(m.id))}
                    className="flex items-center gap-1 rounded-lg bg-surface-3 px-3 py-1.5 text-xs hover:bg-border"
                  >
                    <FileAudio size={14} /> Retranscribe
                  </button>
                </div>
              </div>

              <div className="flex gap-1 border-b border-border px-6">
                {(
                  [
                    ["transcript", "Transcript"],
                    ["summary", "Summary"],
                    ["chat", "Chat"],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    onClick={() => setTab(id)}
                    className={`border-b-2 px-3 py-2 text-sm transition ${
                      tab === id
                        ? "border-accent text-foreground"
                        : "border-transparent text-muted hover:text-foreground"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
                <AnimatePresence mode="wait">
                  {tab === "transcript" && (
                    <motion.div
                      key="transcript"
                      initial={{ opacity: 0, y: 6 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0 }}
                      className="mx-auto flex max-w-3xl flex-col gap-3"
                      data-testid="transcript-panel"
                    >
                      {transcript.segments?.length ? (
                        transcript.segments.map((s) => (
                          <div
                            key={s.id}
                            className={`max-w-[85%] rounded-2xl px-4 py-3 text-sm leading-relaxed ${
                              s.speaker === "me"
                                ? "bubble-me ml-auto"
                                : "bubble-others"
                            }`}
                          >
                            <div className="mb-1 flex items-center gap-2 text-[11px] uppercase tracking-wide text-muted">
                              <span
                                className={
                                  s.speaker === "me" ? "text-me" : "text-others"
                                }
                              >
                                {s.speaker === "me" ? "Me" : "Others"}
                              </span>
                              <span>{formatTs(s.start_ms)}</span>
                            </div>
                            {s.text}
                          </div>
                        ))
                      ) : (
                        <Empty
                          title="No transcript yet"
                          body="Hit Record to capture dual-channel audio. Live lines appear here as Me / Others."
                        />
                      )}
                    </motion.div>
                  )}

                  {tab === "summary" && (
                    <motion.div
                      key="summary"
                      initial={{ opacity: 0, y: 6 }}
                      animate={{ opacity: 1, y: 0 }}
                      className="mx-auto max-w-3xl space-y-6"
                      data-testid="summary-panel"
                    >
                      <Section title="Summary" body={selected.summary || "Not generated yet."} />
                      <Section title="Key points" body={selected.key_points || "—"} />
                      <Section title="Action items" body={selected.action_items || "—"} />
                      <div className="flex flex-wrap gap-2">
                        {["general", "standup", "one_on_one", "client_call"].map((t) => (
                          <button
                            key={t}
                            onClick={() => handleSummarize(t)}
                            className="rounded-full border border-border px-3 py-1 text-xs text-muted hover:border-accent hover:text-accent"
                          >
                            {t.replace("_", " ")}
                          </button>
                        ))}
                      </div>
                    </motion.div>
                  )}

                  {tab === "chat" && (
                    <motion.div
                      key="chat"
                      initial={{ opacity: 0, y: 6 }}
                      animate={{ opacity: 1, y: 0 }}
                      className="mx-auto flex h-full max-w-3xl flex-col"
                      data-testid="chat-panel"
                    >
                      <div className="flex-1 space-y-3 overflow-y-auto pb-4">
                        {chat.length === 0 && (
                          <Empty
                            title="Ask this meeting"
                            body="Questions stay on-device with the local model, or use OpenRouter when configured."
                          />
                        )}
                        {chat.map((m, i) => (
                          <div
                            key={i}
                            className={`rounded-2xl px-4 py-3 text-sm ${
                              m.role === "user"
                                ? "bubble-me ml-auto max-w-[80%]"
                                : "bubble-others max-w-[90%]"
                            }`}
                          >
                            {m.content}
                          </div>
                        ))}
                      </div>
                      <div className="flex gap-2 border-t border-border pt-3">
                        <input
                          value={question}
                          onChange={(e) => setQuestion(e.target.value)}
                          onKeyDown={(e) => e.key === "Enter" && handleChat()}
                          placeholder="Ask about this meeting…"
                          className="flex-1 rounded-xl border border-border bg-surface-2 px-4 py-2.5 text-sm outline-none focus:border-accent"
                        />
                        <button
                          onClick={handleChat}
                          disabled={busy}
                          className="flex items-center gap-1 rounded-xl bg-accent px-4 py-2 text-sm font-medium text-black disabled:opacity-50"
                        >
                          <MessageSquare size={16} /> Send
                        </button>
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            </>
          ) : (
            <div className="flex flex-1 items-center justify-center p-8">
              <Empty
                title="Ready when you are"
                body="Start a recording, import audio, or open a past meeting from the sidebar. Everything stays on this machine."
              />
            </div>
          )}
        </div>
      </main>

      {showSettings && settings && (
        <SettingsPanel
          settings={settings}
          models={models}
          onClose={() => setShowSettings(false)}
          onSave={async (s) => {
            const next = await api.saveSettings(s);
            setSettings(next);
            setModels(await api.listModels());
          }}
          onRefreshModels={async () => setModels(await api.listModels())}
        />
      )}
    </div>
  );
}

function Section({ title, body }: { title: string; body: string }) {
  return (
    <section className="rounded-2xl border border-border bg-surface-2 p-4">
      <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted">
        {title}
      </h2>
      <pre className="whitespace-pre-wrap font-sans text-sm leading-relaxed text-foreground">
        {body}
      </pre>
    </section>
  );
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="mx-auto max-w-md text-center">
      <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-surface-3 text-accent">
        <Search size={22} />
      </div>
      <h2 className="text-lg font-medium">{title}</h2>
      <p className="mt-2 text-sm text-muted">{body}</p>
    </div>
  );
}
