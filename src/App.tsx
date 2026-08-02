import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type UIEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { motion, AnimatePresence, MotionConfig } from "framer-motion";
import { FileAudio, Settings, Sparkles } from "lucide-react";
import {
  api,
  AppSettings,
  AudioDevice,
  formatDuration,
  LiveTranscript,
  MeetingRecord,
  ModelInfo,
  RecorderStatus,
  SearchHit,
  StartGate,
} from "./lib/api";
import { AudioLinesIcon, MicIcon } from "@animateicons/react/lucide";
import { I18nProvider, useI18n } from "./lib/i18n";
import { fadeRise, segmentArrive } from "./lib/motion";
import { applyTheme } from "./lib/theme";
import { Button, FOCUS } from "./components/Button";
import { Markdown } from "./components/Markdown";
import { Tabs } from "./components/Tabs";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { RecordDock } from "./components/RecordDock";
import { RecordTransport } from "./components/RecordTransport";
import { Sidebar } from "./components/Sidebar";
import { SettingsPanel } from "./components/SettingsPanel";
import { Onboarding } from "./components/Onboarding";
import logo from "./assets/logo.png";

type Tab = "transcript" | "summary";

/// Mirrors the accelerator registered in `src-tauri/src/lib.rs`. No command
/// reports it, and a global shortcut nobody can see is a shortcut nobody uses.
const RECORD_SHORTCUT = "Ctrl+Shift+R";

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
        // The stored value overrules the boot cache the moment it resolves —
        // localStorage only exists because this round-trip cannot beat first
        // paint. Anything that is not "dark" is light, so a backend that does
        // not carry the field yet lands on the product default.
        applyTheme(s.theme === "dark" ? "dark" : "light");
      })
      .catch(() => {
        setSettings(null);
      })
      .finally(() => setReady(true));
  }, []);

  if (!ready || !settings) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-fg-muted">
        Vesper
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
  const [transcriptOwner, setTranscriptOwner] = useState<string | null>(null);
  const [status, setStatus] = useState<RecorderStatus | null>(null);
  const [settings, setSettings] = useState<AppSettings>(initialSettings);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  // Both exist so the sidebar can tell "nothing yet" from "nothing at all":
  // `meetings` is empty before the first list resolves and `hits` is empty
  // through the search debounce, and neither means the same as an empty result.
  const [meetingsLoaded, setMeetingsLoaded] = useState(false);
  const [searching, setSearching] = useState(false);
  const [tab, setTab] = useState<Tab>("transcript");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [updateNote, setUpdateNote] = useState<string | null>(null);
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  const [gate, setGate] = useState<StartGate>({ allowed: false });
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [confirmingRecord, setConfirmingRecord] = useState(false);
  const [skipRecordReminder, setSkipRecordReminder] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<MeetingRecord | null>(null);
  const confirmBeforeRecordingRef = useRef(settings.confirm_before_recording);
  confirmBeforeRecordingRef.current = settings.confirm_before_recording;
  const scrollRef = useRef<HTMLDivElement>(null);
  /// Whether the reading column is following the newest line. A ref, not state:
  /// it is written from a scroll handler at up to one frame per pixel, and this
  /// component owns the meetings, the transcript and the recorder status — a
  /// state write here would re-render the whole shell per frame.
  const pinnedRef = useRef(true);
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
      // Store the key, not the sentence: depending on t here made refreshGate
      // change identity on every catalog load, which re-ran the whole startup
      // effect — meetings, models, status, gate and the update check — each time.
      setGate({ allowed: false, reason_key: "gate.unavailable" });
    }
  }, []);

  const refreshMeetings = useCallback(async () => {
    try {
      setMeetings(await api.listMeetings());
    } catch (e) {
      setError(String(e));
    } finally {
      setMeetingsLoaded(true);
    }
  }, []);

  const loadMeeting = useCallback(async (id: string) => {
    setSelectedId(id);
    try {
      const [tr, m] = await Promise.all([
        api.getTranscript(id),
        api.getMeeting(id),
      ]);
      setTranscript(tr);
      // Which meeting the transcript on screen belongs to. `setSelectedId` above
      // ran before this request resolved, so the scroll effect keyed on the id
      // alone fires against the *outgoing* transcript. This lands with the
      // content and gives that effect something that changes when the
      // replacement is actually there.
      setTranscriptOwner(id);
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
            else requestStart();
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
      } finally {
        if (current) setSearching(false);
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

  /// Jump the reading column to its newest line.
  ///
  /// `auto`, never `smooth`: `scrollTo({behavior:"smooth"})` is not reliably
  /// gated by `prefers-reduced-motion`, and with a line landing every 1.2s the
  /// animation would never finish before the next one restarts it.
  const pinToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "auto" });
  }, []);

  function handleContentScroll(e: UIEvent<HTMLDivElement>) {
    const el = e.currentTarget;
    // 48px of slack, not exact equality: at any zoom other than 100% the three
    // numbers are fractional and never cancel out, so `=== 0` would unpin the
    // column permanently on the first wheel tick.
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight <= 48;
  }

  /// Opening a meeting or coming back to the tab starts at the newest line.
  /// Content is deliberately not in here: a reader who scrolled up must stay
  /// where they are while the recording keeps appending.
  useEffect(() => {
    pinnedRef.current = true;
  }, [selectedId, tab]);

  // Keyed on what the transcript *contains*, not on the object: `poll_live_stt`
  // replaces it every 1200ms whether or not a word was added, so keying on
  // `transcript` would yank the column back down on every silent tick.
  // (Indexed rather than `.at(-1)`: the project targets ES2020.)
  const segmentCount = transcript.segments?.length ?? 0;
  const transcriptMark = `${segmentCount}:${
    transcript.segments?.[segmentCount - 1]?.end_ms ?? 0
  }`;
  // `transcriptOwner` is in here as well as the mark. Two meetings can share a
  // segment count and a final `end_ms` — two four-line imports of the same
  // length do — so across that switch the mark is identical, nothing re-runs,
  // and the second transcript opens at the first one's offset. The pane is not
  // remounted either, so the ref callback below does not cover it. Keying on
  // the owner rather than on `selectedId` is what makes this fire *after* the
  // replacement has landed instead of against the outgoing content.
  useEffect(() => {
    if (tab !== "transcript" || !pinnedRef.current) return;
    pinToBottom();
  }, [transcriptMark, tab, transcriptOwner, pinToBottom]);

  /// The pane mounts after the outgoing one has animated away, which is a DOM
  /// change with no render of this component behind it — the effect above
  /// cannot see it. Leaving the tab while lines arrive and coming back has to
  /// land on the newest one, so the pane honours the pin as it lands.
  const pinOnPaneMount = useCallback(
    (node: HTMLDivElement | null) => {
      if (node && pinnedRef.current) pinToBottom();
    },
    [pinToBottom],
  );

  /// The dock and the hotkey both go through here, so the reminder cannot be
  /// skipped by starting a recording from the keyboard.
  ///
  /// Reads the preference from a ref rather than the closed-over value: the
  /// hotkey listener is registered once on mount, so it would otherwise keep the
  /// setting as it was at startup and go on asking after the user opted out.
  function requestStart() {
    if (confirmBeforeRecordingRef.current) {
      setConfirmingRecord(true);
      return;
    }
    void handleStart();
  }

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
    // Raised here rather than in the debounce effect so it lands in the same
    // render as the new query. An effect runs after paint, which is one frame
    // of "no matches" — the exact flash the flag exists to prevent.
    setSearching(q.trim().length > 0);
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

  /// Everything the workspace column is showing, and nothing else. The meeting
  /// list, the search, the recorder, the gate, the settings and the model list
  /// all survive: clearing the workspace is not resetting the app.
  function clearWorkspace() {
    setSelectedId(null);
    setTranscript({ segments: [] });
    setTranscriptOwner(null);
  }

  /// Back to the empty screen, on demand — there was no way to get there
  /// without deleting the meeting you were reading.
  ///
  /// No confirmation, in either state. Not recording, nothing is lost.
  /// Recording, the capture keeps running and the header transport keeps
  /// showing it, so there is nothing to warn about either.
  function startNewMeeting() {
    clearWorkspace();
    setTab("transcript");
    setError(null);
  }

  async function handleDelete(id: string) {
    // Dismiss first. Leaving the dialog up after the meeting is gone offers a
    // Delete button for something that no longer exists.
    setPendingDelete(null);
    try {
      await api.deleteMeeting(id);
      if (selectedId === id) clearWorkspace();
      await refreshMeetings();
    } catch (e) {
      setError(String(e));
    }
  }


  return (
    <div className="flex h-full flex-col bg-background text-fg">
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

      {/* Window chrome. It spans the sidebar and the content, and it is the
          canvas: no fill of its own, and no border either — the 8px gutter
          around the card below is the separation, and a hairline 8px from the
          card's own border is two hard divides in a row.

          `data-tauri-drag-region` goes on this element and on no child: Tauri's
          handler tests `event.target`, so a click on a button inside is not a
          drag, and its built-in double-click-to-maximise is the Windows
          behaviour to inherit rather than reimplement.

          48px, not 56: the row has to fit an `h-8` control with 8px of
          clearance, and 48 plus the card's 8px gutter is the same chrome
          budget the `h-14` header was already spending.

          Three columns, not flex with mx-auto. In a flex row `mx-auto` centres
          an item between its siblings, so an identity block wider than the
          actions block pushed the record control off-centre — and it moved
          again whenever the recording state changed the width of either side.
          The centre column is anchored to the header, and its contents can
          grow and shrink without dragging anything with them. The height is
          fixed for the same reason: the transport is the widest thing that
          lands in the centre column, and it must not be able to grow the
          header. */}
      <header
        data-tauri-drag-region
        className="grid h-12 shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 pl-4 pr-2"
      >
        <div className="flex items-center gap-2">
          <img src={logo} alt="" className="h-8 w-8 rounded-md" />
          {/* Flat, not a gradient: the gradient made the brightest, most
              saturated thing on screen a piece of chrome the user cannot act
              on, and it was the only reason --color-accent-2 existed. */}
          <span className="text-lg font-semibold text-fg">{t("app.name")}</span>
        </div>

        {/* Centre column: the transport for a recording in progress. It used
            to be a badge that only said "Recording" while Stop lived in the
            dock — which is on the empty screen, and starting a recording
            leaves that screen. The controls follow the state they control. */}
        <div className="flex items-center justify-center">
          <AnimatePresence>
            {status?.recording && (
              <motion.div key="transport" {...fadeRise}>
                <RecordTransport
                  status={status}
                  busy={busy}
                  onStop={handleStop}
                  onPauseResume={handlePauseResume}
                />
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        <div className="flex items-center justify-end gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setShowSettings(true)}
            title={t("nav.settings")}
            aria-label={t("nav.settings")}
          >
            <Settings size={16} />
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <Sidebar
          meetings={meetings}
          selectedId={selectedId}
          query={query}
          hits={hits}
          loading={!meetingsLoaded}
          searching={searching}
          onSearch={handleSearch}
          onSelect={loadMeeting}
          onDelete={(id) => setPendingDelete(meetings.find((m) => m.id === id) ?? null)}
          onImport={handleImport}
          onNew={startNewMeeting}
        />

        <main className="flex min-w-0 flex-1 flex-col">
          {/* THE CARD. One JSX element and not a component, because it has
              exactly one consumer. `overflow-hidden` clips its own descendants
              into the 12px corners — the banners' full-bleed borders, the
              scroller, the dock — while `<main>` above must NOT clip, or the
              light-mode lift shadow has nowhere to go. The `m-2` gutter is
              symmetric on four sides and sized to contain that shadow, whose
              maximum extent is 6px. */}
          <div className="relative m-2 flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border bg-surface-1 shadow-lift">
            {/* The card's first children, not a strip on the canvas above it:
                out there they would run the full window width and break the
                card's top alignment. In here the radius clips them and they
                read at content level. */}
            {error && (
              <div className="border-b border-danger/30 bg-danger/10 px-6 py-2 text-sm text-danger">
                {error}
                <Button variant="link" className="ml-3" onClick={() => setError(null)}>
                  {t("action.dismiss")}
                </Button>
              </div>
            )}
            {pendingUpdate && (
              <div className="flex flex-wrap items-center gap-3 border-b border-accent/30 bg-accent/10 px-6 py-2 text-sm text-accent">
                <span>{t("update.available").replace("{version}", pendingUpdate.version)}</span>
                <Button
                  size="xs"
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
                >
                  {t("update.install")}
                </Button>
                <Button variant="link" onClick={() => setPendingUpdate(null)}>
                  {t("update.later")}
                </Button>
              </div>
            )}
            {updateNote && (
              <div className="border-b border-accent/30 bg-accent/10 px-6 py-2 text-sm text-accent">
                {updateNote}
              </div>
            )}

            {selected ? (
              <>
                {/* `max-w-pane` (1024px) is the card's content column. The
                    meeting header and the tab bar bound to it so the rules they
                    carry stop on the same two vertical edges, instead of running
                    to the card's own border and putting a full-width divide
                    inside a rounded corner. */}
                <div className="mx-auto flex w-full max-w-pane items-center justify-between border-b border-border px-6 py-3">
                  <div>
                    <h1 className="text-base font-medium">{selected.title}</h1>
                    <p className="text-xs text-fg-muted">
                      {selected.status} · {formatDuration(selected.duration_ms)}
                    </p>
                  </div>
                  {/* Five siblings used to read as five equal actions. Summarize
                      is the one primary; MD/PDF/DOCX are one action with a format
                      parameter, so they sit inside a single bordered group.
                      The inset is horizontal only: a `p-1` shell would stand 34px
                      tall between two 24px chips, and a group that is half again
                      the height of its neighbours reads as a different tier of
                      control rather than a bracket around three of them. */}
                  <div className="flex items-center gap-2">
                    <Button size="xs" onClick={() => handleSummarize("general")}>
                      <Sparkles size={16} /> {t("action.summarize")}
                    </Button>
                    <div className="flex items-center gap-1 rounded-md border border-border px-1">
                      {(["md", "pdf", "docx"] as const).map((format) => (
                        <Button
                          key={format}
                          size="xs"
                          variant="ghost"
                          onClick={() => handleExport(format)}
                        >
                          {format.toUpperCase()}
                        </Button>
                      ))}
                    </div>
                    <Button
                      size="xs"
                      variant="secondary"
                      onClick={() =>
                        selectedId &&
                        api.retranscribe(selectedId).then((m) => loadMeeting(m.id))
                      }
                    >
                      <FileAudio size={16} /> {t("action.retranscribe")}
                    </Button>
                  </div>
                </div>

                <Tabs
                  idPrefix="content"
                  className="mx-auto w-full max-w-pane px-6"
                  value={tab}
                  onChange={setTab}
                  items={[
                    { id: "transcript", label: t("tab.transcript") },
                    { id: "summary", label: t("tab.summary") },
                  ]}
                />

                {/* The scroll container is the tab panel: the two panes swap
                    inside it, so the id follows the selected tab rather than
                    living on a pane that unmounts. */}
                <div
                  ref={scrollRef}
                  onScroll={handleContentScroll}
                  data-scroll
                  id={`content-panel-${tab}`}
                  role="tabpanel"
                  aria-labelledby={`content-tab-${tab}`}
                  className="min-h-0 flex-1 overflow-y-auto px-6 pb-6 pt-4"
                >
                  <AnimatePresence mode="wait">
                    {tab === "transcript" && (
                      <motion.div
                        key="transcript"
                        ref={pinOnPaneMount}
                        {...fadeRise}
                        className="mx-auto max-w-reading"
                        data-testid="transcript-panel"
                      >
                        {transcript.segments?.length ? (
                          // `initial={false}` so opening a past meeting does not
                          // blur-and-fade three hundred rows at once: only the
                          // segments that land after the pane is up animate, which
                          // is the handful per minute `segmentArrive` is sized for.
                          <AnimatePresence initial={false}>
                            {transcript.segments.map((s, i) => {
                              // A run of consecutive lines from one speaker is one
                              // utterance: it carries a single timestamp and a
                              // single name, and only the gap says where it ends.
                              const opens =
                                i === 0 ||
                                transcript.segments[i - 1].speaker !== s.speaker;
                              return (
                                <motion.div
                                  key={s.id}
                                  {...segmentArrive}
                                  className={`flex gap-4 ${
                                    i === 0 ? "" : opens ? "mt-4" : "mt-1"
                                  }`}
                                >
                                  <span className="w-14 shrink-0 tabular-nums text-2xs text-fg-faint">
                                    {opens ? formatDuration(s.start_ms) : ""}
                                  </span>
                                  <span
                                    aria-hidden
                                    className={`w-0.5 shrink-0 self-stretch rounded-full ${
                                      s.speaker === "me" ? "bg-accent" : "bg-others"
                                    }`}
                                  />
                                  <div className="min-w-0 flex-1">
                                    {opens && (
                                      <div className="text-2xs text-fg-subtle">
                                        {s.speaker === "me"
                                          ? t("speaker.me")
                                          : t("speaker.others")}
                                      </div>
                                    )}
                                    <p className="text-base leading-relaxed">{s.text}</p>
                                  </div>
                                </motion.div>
                              );
                            })}
                          </AnimatePresence>
                        ) : (
                          <Empty
                            icon={<AudioLinesIcon size={22} />}
                            title={t("empty.transcript")}
                            body={t("empty.transcript_body")}
                            action={
                              <Button
                                size="md"
                                variant="secondary"
                                onClick={handleImport}
                                disabled={busy}
                              >
                                {t("nav.import")}
                              </Button>
                            }
                          />
                        )}
                      </motion.div>
                    )}

                    {tab === "summary" && (
                      <motion.div
                        key="summary"
                        {...fadeRise}
                        className="mx-auto max-w-reading space-y-6"
                        data-testid="summary-panel"
                      >
                        {selected.summary ||
                        selected.key_points ||
                        selected.action_items ? (
                          <>
                            <Section title={t("section.summary")} body={selected.summary || "—"} />
                            <Section title={t("section.key_points")} body={selected.key_points || "—"} />
                            <Section title={t("section.action_items")} body={selected.action_items || "—"} />
                          </>
                        ) : (
                          // Three cards each holding one em-dash and nothing to
                          // act on is an empty state. This renders it as one.
                          <Empty
                            icon={<Sparkles size={22} />}
                            title={t("empty.summary")}
                            body={t("empty.summary_body")}
                            action={
                              <Button
                                size="md"
                                onClick={() => handleSummarize("general")}
                                disabled={busy}
                              >
                                <Sparkles size={16} /> {t("action.summarize")}
                              </Button>
                            }
                          />
                        )}
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
                  action={
                    // No Record here. The dock at the foot of this same screen is
                    // the record button — putting a second one in the middle gave
                    // the empty screen two primary actions that do the same thing,
                    // eight hundred pixels apart. The shortcut still belongs here,
                    // where there is room to name it.
                    <div className="flex items-center justify-center gap-3">
                      <Button
                        size="md"
                        variant="secondary"
                        onClick={handleImport}
                        disabled={busy}
                      >
                        {t("nav.import")}
                      </Button>
                      {!status?.recording && (
                        <span className="flex items-center gap-2 text-2xs text-fg-subtle">
                          {t("record.start")}
                          <kbd className="inline-flex h-6 items-center rounded-xs border border-border bg-surface-2 px-2 font-sans text-2xs">
                            {RECORD_SHORTCUT}
                          </kbd>
                        </span>
                      )}
                    </div>
                  }
                />
              </div>
            )}

            {/* The empty screen only. With a meeting open the transcript and the
                summary own this column, and a Record button hanging over them
                offers to start a second recording on top of the one the header is
                already showing. */}
            <AnimatePresence>
              {!selected && !status?.recording && (
                <RecordDock
                  key="dock"
                  gate={gate}
                  busy={busy}
                  devices={devices}
                  micDeviceId={settings.mic_device_id}
                  systemDeviceId={settings.system_device_id}
                  onStart={requestStart}
                  onOpenSettings={() => setShowSettings(true)}
                />
              )}
            </AnimatePresence>
          </div>
        </main>
      </div>

      <AnimatePresence>
        {confirmingRecord && (
          <ConfirmDialog
            title={t("confirm.record_title")}
            body={t("confirm.record_body")}
            confirmLabel={t("confirm.record_accept")}
            extra={
              <label className="flex items-center gap-2 text-xs text-fg-muted">
                <input
                  type="checkbox"
                  checked={skipRecordReminder}
                  onChange={(e) => setSkipRecordReminder(e.target.checked)}
                  className={FOCUS}
                />
                {t("confirm.record_dont_ask")}
              </label>
            }
            onCancel={() => {
              setConfirmingRecord(false);
              // A ticked box that survives a cancel would silently opt the user
              // out the next time they press Start.
              setSkipRecordReminder(false);
            }}
            onConfirm={async () => {
              setConfirmingRecord(false);
              if (skipRecordReminder) {
                // Persist before recording starts: the preference is the user's
                // answer to the question they were just asked, and losing it
                // would ask again next time as if they had not replied.
                const next = { ...settings, confirm_before_recording: false };
                try {
                  const saved = await api.saveSettings(next);
                  setSettings(saved);
                  onSettingsChange(saved);
                } catch (e) {
                  // Do not record. `handleStart` clears the error banner, so
                  // carrying on would swallow the failure and leave the user
                  // believing a preference was saved that was not.
                  setError(String(e));
                  setSkipRecordReminder(false);
                  return;
                }
              }
              setSkipRecordReminder(false);
              await handleStart();
            }}
          />
        )}
      </AnimatePresence>

      <AnimatePresence>
        {pendingDelete && (
          <ConfirmDialog
            danger
            title={t("confirm.delete_title")}
            body={t("confirm.delete_body")}
            confirmLabel={t("confirm.delete_accept")}
            onCancel={() => setPendingDelete(null)}
            onConfirm={() => handleDelete(pendingDelete.id)}
          />
        )}
      </AnimatePresence>

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
    <section className="rounded-lg border border-border bg-surface-2 p-4">
      <h2 className="mb-2 text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
        {title}
      </h2>
      {/* Not a `<pre>`. The summariser is asked for markdown, so the app's
          headline deliverable — the first thing read after a recording stops —
          was reaching the screen as literal `**` and `- `. */}
      <Markdown text={body} />
    </section>
  );
}

/// The icon is a parameter because the empty states mean different things: one
/// is waiting for you to record, one is a transcript that has not arrived yet,
/// one is a meeting nobody has summarized. They all used to show a magnifying
/// glass, which described none of them.
///
/// `action` is what makes these states empty rather than dead. Every screen in
/// the app now names the way out of itself.
function Empty({
  icon,
  title,
  body,
  action,
}: {
  icon: ReactNode;
  title: string;
  body: string;
  action?: ReactNode;
}) {
  return (
    <div className="mx-auto max-w-md text-center">
      {/* Not `text-accent`: the accent belongs to the primary action below,
          and an empty state's icon is decoration. */}
      <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-lg bg-surface-2 text-fg-subtle">
        {icon}
      </div>
      <h2 className="text-lg font-semibold">{title}</h2>
      <p className="mt-2 text-base leading-relaxed text-fg-muted">{body}</p>
      {action && <div className="mt-6">{action}</div>}
    </div>
  );
}
