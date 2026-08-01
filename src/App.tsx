import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { motion, AnimatePresence, MotionConfig } from "framer-motion";
import { FileAudio, MessageSquare, Settings, Sparkles } from "lucide-react";
import {
  api,
  AppSettings,
  AudioDevice,
  ChatMessage,
  formatDuration,
  formatTs,
  LiveTranscript,
  MeetingRecord,
  ModelInfo,
  RecorderStatus,
  SearchHit,
  StartGate,
} from "./lib/api";
import { AudioLinesIcon, MicIcon } from "@animateicons/react/lucide";
import { I18nProvider, useI18n } from "./lib/i18n";
import { fadeRise } from "./lib/motion";
import { RecordDock } from "./components/RecordDock";
import { Sidebar } from "./components/Sidebar";
import { SettingsPanel } from "./components/SettingsPanel";
import { Onboarding } from "./components/Onboarding";
import logo from "./assets/logo.png";

type Tab = "transcript" | "summary" | "chat";

export default function App() {
  const [bootLocale, setBootLocale] = useState("en");
  const [ready, setReady] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setBootLocale(s.ui_locale || "en");
      })
      .catch(() => {
        setSettings(null);
      })
      .finally(() => setReady(true));
  }, []);

  if (!ready || !settings) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-muted">
        Loading Vesper…
      </div>
    );
  }

  return (
    // reducedMotion="user" honours the OS setting for everyone below this point,
    // so no individual animation has to remember to check it.
    <MotionConfig reducedMotion="user">
      <I18nProvider initialLocale={bootLocale}>
        <AppShell
          initialSettings={settings}
          onSettingsChange={setSettings}
        />
      </I18nProvider>
    </MotionConfig>
  );
}

function AppShell({
  initialSettings,
  onSettingsChange,
}: {
  initialSettings: AppSettings;
  onSettingsChange: (s: AppSettings) => void;
}) {
  const { t, setLocale } = useI18n();
  const [meetings, setMeetings] = useState<MeetingRecord[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<LiveTranscript>({ segments: [] });
  const [status, setStatus] = useState<RecorderStatus | null>(null);
  const [settings, setSettings] = useState<AppSettings>(initialSettings);
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
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  const [gate, setGate] = useState<StartGate>({ allowed: false });
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [showOnboarding, setShowOnboarding] = useState(
    !initialSettings.onboarding_complete,
  );

  const selected = useMemo(
    () => meetings.find((m) => m.id === selectedId) ?? null,
    [meetings, selectedId],
  );

  const refreshGate = useCallback(async () => {
    try {
      setGate(await api.canRecord());
    } catch {
      setGate({ allowed: false, reason: "Gate unavailable" });
    }
  }, []);

  const refreshMeetings = useCallback(async () => {
    try {
      setMeetings(await api.listMeetings());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadMeeting = useCallback(async (id: string) => {
    setSelectedId(id);
    try {
      const [tr, c, m] = await Promise.all([
        api.getTranscript(id),
        api.listChat(id),
        api.getMeeting(id),
      ]);
      setTranscript(tr);
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

  // Device enumeration is allowed to fail — a machine with no capture device
  // still runs the app. Kept apart from the startup refresh so a rejection here
  // cannot skip the recorder status and the gate, which would leave the dock
  // disabled with no way back.
  const refreshDevices = useCallback(async () => {
    try {
      setDevices(await api.listDevices());
    } catch {
      /* leave the previous list in place */
    }
  }, []);

  useEffect(() => {
    (async () => {
      void refreshDevices();
      try {
        await refreshMeetings();
        setModels(await api.listModels());
        setStatus(await api.recorderStatus());
        await refreshGate();
      } catch (e) {
        setError(String(e));
      }
      try {
        // Only ever offered, never applied on its own: installing and relaunching
        // without asking can throw away a recording in progress, and silently
        // swapping the binary of a privacy tool is not ours to decide.
        const update = await check();
        if (update) setPendingUpdate(update);
      } catch {
        /* no release endpoint yet */
      }
    })();
  }, [refreshMeetings, refreshGate, refreshDevices]);

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

  // Debounced so typing does not fire one query per keystroke. Every run
  // supersedes the one before it, and a stale response that arrives after the
  // query moved on is discarded rather than painted over the newer results.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits([]);
      return;
    }
    let current = true;
    const timer = window.setTimeout(async () => {
      try {
        const found = await api.search(q);
        if (current) setHits(found);
      } catch (e) {
        if (current) setError(String(e));
      }
    }, 200);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [query]);

  useEffect(() => {
    if (!status?.recording) return;
    const id = window.setInterval(async () => {
      try {
        setStatus(await api.recorderStatus());
        setTranscript(await api.pollLiveStt());
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
      const g = await api.canRecord();
      setGate(g);
      if (!g.allowed) {
        setError(g.reason || t("gate.local_stt"));
        return;
      }
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
      await refreshGate();
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

  function handleSearch(q: string) {
    setQuery(q);
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
        filters: [{ name: "Audio", extensions: ["wav", "mp3", "m4a", "webm", "ogg", "flac"] }],
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
      {showOnboarding && (
        <Onboarding
          settings={settings}
          onDone={async (s) => {
            setSettings(s);
            onSettingsChange(s);
            setLocale(s.ui_locale);
            setShowOnboarding(false);
            setModels(await api.listModels());
            await refreshGate();
          }}
        />
      )}

      <Sidebar
        meetings={meetings}
        selectedId={selectedId}
        query={query}
        hits={hits}
        onSearch={handleSearch}
        onSelect={loadMeeting}
        onDelete={handleDelete}
        onImport={handleImport}
      />

      <main className="flex min-w-0 flex-1 flex-col">
        {/* Three columns, not flex with mx-auto. In a flex row `mx-auto` centres an
            item between its siblings, so an identity block wider than the actions
            block pushed the record control off-centre — and it moved again
            whenever the recording state changed the width of either side. The
            centre column is now anchored to the header, and its contents can grow
            and shrink without dragging anything with them. */}
        <header className="grid grid-cols-[1fr_auto_1fr] items-center gap-4 border-b border-border bg-surface px-6 py-4">
          <div className="flex items-center gap-2.5">
            <img src={logo} alt="" className="h-8 w-8 rounded-lg" />
            {/* Negative tracking on the wordmark: at this size the default
                spacing reads loose. */}
            <span className="bg-gradient-to-r from-accent to-accent-2 bg-clip-text text-lg font-semibold tracking-[-0.02em] text-transparent">
              {t("app.name")}
            </span>
          </div>

          {/* Centre column reserved for status. The recording controls moved to
              the dock at the bottom of the content column, where nothing beside
              them can change their position. */}
          <div className="flex items-center justify-center">
            {status?.recording && (
              <span
                data-testid="recording-badge"
                className="flex items-center gap-2 rounded-full bg-danger/10 px-3 py-1 text-xs text-danger"
              >
                <span className="h-1.5 w-1.5 rounded-full bg-danger" aria-hidden />
                {status.paused ? t("record.paused_badge") : t("record.recording_badge")}
              </span>
            )}
          </div>

          <div className="flex items-center justify-end gap-3">
            <button
              onClick={() => setShowSettings(true)}
              className="rounded-lg p-2 text-muted transition-colors hover:bg-surface-3 hover:text-foreground focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-background),0_0_0_4px_var(--color-accent)]"
              title={t("nav.settings")}
              aria-label={t("nav.settings")}
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
        {pendingUpdate && (
          <div className="flex flex-wrap items-center gap-3 border-b border-accent/30 bg-accent/10 px-6 py-2 text-sm text-accent">
            <span>{t("update.available").replace("{version}", pendingUpdate.version)}</span>
            <button
              data-testid="update-install"
              onClick={async () => {
                const update = pendingUpdate;
                setPendingUpdate(null);
                try {
                  setUpdateNote(t("update.installing"));
                  await update.downloadAndInstall();
                  setUpdateNote(t("update.relaunching"));
                  await relaunch();
                } catch (e) {
                  setUpdateNote(null);
                  setError(String(e));
                  // The check only runs once, on mount. Without putting the offer
                  // back, a failed download means no retry until the app restarts.
                  setPendingUpdate(update);
                }
              }}
              className="rounded-lg bg-accent px-3 py-1 text-xs font-medium text-black hover:brightness-110"
            >
              {t("update.install")}
            </button>
            <button
              onClick={() => setPendingUpdate(null)}
              className="text-xs underline"
            >
              {t("update.later")}
            </button>
          </div>
        )}
        {updateNote && (
          <div className="border-b border-accent/30 bg-accent/10 px-6 py-2 text-sm text-accent">
            {updateNote}
          </div>
        )}

        <div className="relative flex min-h-0 flex-1 flex-col">
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
                    onClick={() =>
                      selectedId &&
                      api.retranscribe(selectedId).then((m) => loadMeeting(m.id))
                    }
                    className="flex items-center gap-1 rounded-lg bg-surface-3 px-3 py-1.5 text-xs hover:bg-border"
                  >
                    <FileAudio size={14} /> Retranscribe
                  </button>
                </div>
              </div>

              <div className="flex gap-1 border-b border-border px-6">
                {(
                  [
                    ["transcript", t("tab.transcript")],
                    ["summary", t("tab.summary")],
                    ["chat", t("tab.chat")],
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

              <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-36 pt-4">
                <AnimatePresence mode="wait">
                  {tab === "transcript" && (
                    <motion.div
                      key="transcript"
                      {...fadeRise}
                      exit={{ opacity: 0 }}
                      className="mx-auto flex max-w-3xl flex-col gap-3"
                      data-testid="transcript-panel"
                    >
                      {transcript.segments?.length ? (
                        transcript.segments.map((s) => (
                          <div
                            key={s.id}
                            className={`max-w-[85%] rounded-2xl px-4 py-3 text-sm leading-relaxed ${
                              s.speaker === "me" ? "bubble-me ml-auto" : "bubble-others"
                            }`}
                          >
                            <div className="mb-1 flex items-center gap-2 text-[11px] uppercase tracking-wide text-muted">
                              <span
                                className={
                                  s.speaker === "me" ? "text-me" : "text-others"
                                }
                              >
                                {s.speaker === "me"
                                  ? t("speaker.me")
                                  : t("speaker.others")}
                              </span>
                              <span>{formatTs(s.start_ms)}</span>
                            </div>
                            {s.text}
                          </div>
                        ))
                      ) : (
                        <Empty
                          icon={<AudioLinesIcon size={22} />}
                          title={t("empty.transcript")}
                          body={t("empty.transcript_body")}
                        />
                      )}
                    </motion.div>
                  )}

                  {tab === "summary" && (
                    <motion.div
                      key="summary"
                      {...fadeRise}
                      className="mx-auto max-w-3xl space-y-6"
                      data-testid="summary-panel"
                    >
                      <Section title="Summary" body={selected.summary || "—"} />
                      <Section title="Key points" body={selected.key_points || "—"} />
                      <Section title="Action items" body={selected.action_items || "—"} />
                    </motion.div>
                  )}

                  {tab === "chat" && (
                    <motion.div
                      key="chat"
                      {...fadeRise}
                      className="mx-auto flex h-full max-w-3xl flex-col"
                      data-testid="chat-panel"
                    >
                      <div className="flex-1 space-y-3 overflow-y-auto pb-4">
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
                          placeholder="…"
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
                icon={<MicIcon size={22} />}
                title={t("empty.ready")}
                body={t("empty.ready_body")}
              />
            </div>
          )}

          <RecordDock
            status={status}
            gate={gate}
            busy={busy}
            devices={devices}
            micDeviceId={settings.mic_device_id}
            systemDeviceId={settings.system_device_id}
            onStart={handleStart}
            onStop={handleStop}
            onPauseResume={handlePauseResume}
          />
        </div>
      </main>

      {/* AnimatePresence so the drawer animates out as well as in — without it
          the exit is a cut. */}
      <AnimatePresence>
        {showSettings && (
          <SettingsPanel
            settings={settings}
            models={models}
            onClose={() => setShowSettings(false)}
            onSave={async (s) => {
              const next = await api.saveSettings(s);
              setSettings(next);
              onSettingsChange(next);
              setModels(await api.listModels());
              // The dock resolves device names from this list, so a device
              // plugged in after launch would otherwise show as the old default.
              // Failure here must not report the save as failed — it already
              // succeeded.
              void refreshDevices();
              await refreshGate();
            }}
            onRefreshModels={async () => {
              setModels(await api.listModels());
              await refreshGate();
            }}
          />
        )}
      </AnimatePresence>
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

/// The icon is a parameter because the two empty states mean different things:
/// one is waiting for you to record, the other is a transcript that has not
/// arrived yet. Both used to show a magnifying glass, which described neither.
function Empty({
  icon,
  title,
  body,
}: {
  icon: ReactNode;
  title: string;
  body: string;
}) {
  return (
    <div className="mx-auto max-w-md text-center">
      <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-surface-3 text-accent">
        {icon}
      </div>
      <h2 className="text-lg font-medium tracking-[-0.01em]">{title}</h2>
      <p className="mt-2 text-sm leading-relaxed text-muted">{body}</p>
    </div>
  );
}
