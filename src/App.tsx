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
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { motion, AnimatePresence, MotionConfig } from "framer-motion";
import { clsx } from "clsx";
import { House, Moon, PanelLeftClose, PanelLeftOpen, Settings, Sparkles, Sun, TriangleAlert } from "lucide-react";
import {
  api,
  AppSettings,
  AudioDevice,
  Deaf,
  formatDuration,
  LiveTranscript,
  MeetingProgress,
  MeetingRecord,
  ModelInfo,
  RecorderStatus,
  SearchHit,
  ShortcutStatus,
  Speaker,
  StartGate,
} from "./lib/api";
import { AudioLinesIcon, MicIcon } from "@animateicons/react/lucide";
import { formatMeetingDateTime } from "./lib/datetime";
import { I18nProvider, useI18n } from "./lib/i18n";
import { fadeRise, segmentArrive, transition } from "./lib/motion";
import { applyTheme } from "./lib/theme";
import { Button, FOCUS, PANEL } from "./components/Button";
import { Markdown } from "./components/Markdown";
import { Tabs } from "./components/Tabs";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { RecoveryDialog } from "./components/RecoveryDialog";
import { CopyButton } from "./components/CopyButton";
import { SummaryHistory } from "./components/SummaryHistory";
import { ChatPanel } from "./components/ChatPanel";
import { SummarizeButton } from "./components/SummarizeButton";
import { ActionItems } from "./components/ActionItems";
import { NotesPanel } from "./components/NotesPanel";
import { EditableLine } from "./components/EditableLine";
import { AudioPlayer } from "./components/AudioPlayer";
import { ProcessingStatus } from "./components/ProcessingStatus";
import { RecordDock } from "./components/RecordDock";
import { ContextBar } from "./components/ContextBar";
import { EgressBadge } from "./components/EgressBadge";
import { RecordTransport } from "./components/RecordTransport";
import { Sidebar } from "./components/Sidebar";
import { WindowControls } from "./components/WindowControls";
import { SettingsPanel } from "./components/SettingsPanel";
import { Onboarding } from "./components/Onboarding";
import logo from "./assets/logo.png";

type Tab = "transcript" | "summary" | "notes" | "chat";

/** Move the window, restoring it first if it is maximised.
 *
 *  Tauri's own drag region calls `startDragging` and stops there, which a
 *  maximised window ignores — so the titlebar felt dead in the state the app
 *  starts in. Windows restores the window and hands it to the cursor, and that
 *  is the behaviour to match rather than invent around. */
async function dragWindow(stillPressed: () => boolean) {
  const win = getCurrentWindow();
  // Each step is a round trip to the backend, and the button can come up
  // during any of them. Without these checks a flick — four pixels and
  // release — unmaximised the window and then handed it to the OS move loop
  // with nothing held down, so it followed the cursor until the next click.
  if (await win.isMaximized()) {
    if (!stillPressed()) return;
    await win.unmaximize();
  }
  if (!stillPressed()) return;
  await win.startDragging();
}

/// The sidebar's width, in pixels. Named because two places have to agree on
/// it: the backdrop that draws it and the card that slides exactly that far.
const SIDEBAR_WIDTH = 288;

/// Whether a key event is the accelerator the backend registered.
///
/// Parsed from the same string the backend holds rather than hardcoded, so the
/// focused-window shortcut and the global one cannot be different keys. `e.code`
/// and not `e.key`: layout-independent, and the same `KeyR` the backend
/// registers.
///
/// An accelerator this cannot parse matches nothing. The global shortcut still
/// works — that one is registered by the OS, not by this — so the cost is the
/// focused case, which is the milder half.
function matchesAccelerator(e: KeyboardEvent, accelerator?: string): boolean {
  if (!accelerator) return false;
  const parts = accelerator.split("+").map((p) => p.trim().toLowerCase());
  const key = parts[parts.length - 1];
  const want = {
    ctrl: parts.includes("ctrl") || parts.includes("control"),
    shift: parts.includes("shift"),
    alt: parts.includes("alt"),
  };
  if (e.ctrlKey !== want.ctrl || e.shiftKey !== want.shift || e.altKey !== want.alt) {
    return false;
  }
  // `Ctrl+Shift+R` -> KeyR, `Ctrl+Shift+F9` -> F9, `Ctrl+Alt+Space` -> Space.
  const code =
    key.length === 1 && key >= "a" && key <= "z"
      ? `Key${key.toUpperCase()}`
      : key === "space"
        ? "Space"
        : key.toUpperCase();
  return e.code === code;
}


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
  /// Where the selected meeting's recording is, once the backend has proven the
  /// row points inside its own directory.
  ///
  /// `undefined` while the answer is still in flight, which is a different thing
  /// from `null`, "there is nothing to play". Without the distinction the note
  /// about a missing recording flashes over every meeting on the way in.
  const [audioPath, setAudioPath] = useState<string | null | undefined>(
    undefined,
  );
  /// The player's element, held here because the transcript seeks through it and
  /// the transcript is rendered by this component rather than by the player.
  const audioRef = useRef<HTMLAudioElement | null>(null);
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
  // Kept apart from `error`: a live transcription that keeps failing would
  // otherwise reappear every 1.2s under a Dismiss the user has already pressed.
  // This one clears itself the moment a tick succeeds.
  const [liveSttError, setLiveSttError] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [updateNote, setUpdateNote] = useState<string | null>(null);
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  /// Whether the bytes are already on disk, which is what turns the offer from
  /// "install" — a download the user waits through — into "restart".
  const [updateReady, setUpdateReady] = useState(false);
  const [gate, setGate] = useState<StartGate>({ allowed: false });
  /// What the backend says about the global accelerator. `null` until it
  /// answers, and permanently `null` on a build whose backend does not report
  /// it — in which case the app names the key it listens for and claims nothing
  /// about the OS.
  const [shortcut, setShortcut] = useState<ShortcutStatus | null>(null);
  /// The room has been quiet long enough that Vesper has asked whether anybody
  /// is still there. Cleared by an answer, by somebody speaking, or by the stop
  /// that follows an unanswered question.
  const [silent, setSilent] = useState(false);
  // A channel that was asked to record and has heard nothing at all. Distinct
  // from `silent`, which is a room where nobody is speaking: this one is a dead
  // input, and the recording is being lost while the banner is on screen.
  const [deaf, setDeaf] = useState<Deaf>({ me: false, others: false });
  // Picks the catalogue no longer offers, moved to the nearest tier at
  // startup. Read once — the command drains what it returns.
  const [retired, setRetired] = useState<[string, string][]>([]);
  /// The last phase the backend reported for a meeting being finished. `null`
  /// until the first event, and permanently `null` on a build whose backend
  /// does not emit them — in which case none of the UI below renders and Stop
  /// behaves as it did.
  const [progress, setProgress] = useState<MeetingProgress | null>(null);
  /// The meeting whose re-read failed, kept past the phase that reported it.
  /// `meeting://progress` is one channel and the walk carries on: the summary
  /// phases land after this one and would take the outcome off the screen
  /// before anybody had read it. An id, so it cannot be shown over a different
  /// meeting the user has opened since.
  const [finalPassFailedId, setFinalPassFailedId] = useState<string | null>(
    null,
  );
  // The meeting being summarised, not a flag. Not `busy` either: that one is
  // also raised by starting, stopping and importing, and it would put the
  // summary pane to work over a recording that has not been transcribed yet.
  // An id rather than a boolean because the sidebar stays live while the model
  // writes — select another meeting mid-summary and a flag would claim that one
  // was being worked on too.
  const [summarizingId, setSummarizingId] = useState<string | null>(null);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [confirmingRecord, setConfirmingRecord] = useState(false);
  const [skipRecordReminder, setSkipRecordReminder] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<MeetingRecord | null>(null);
  /// Meetings the app was recording when it last stopped running, still waiting
  /// to be answered for. A queue rather than one: a machine that lost power
  /// twice has two, and each one is its own decision. The head is on screen and
  /// answering it uncovers the next.
  const [interrupted, setInterrupted] = useState<MeetingRecord[]>([]);
  /// Whether the meeting header's title is being edited. A generated title is a
  /// guess, and a guess the user cannot correct is worse than a date.
  const [renaming, setRenaming] = useState(false);
  /// The segment whose turn header is being renamed, or null. The segment and
  /// not the channel: a channel names every turn it opens, and keying on it
  /// would turn all of them into an input at once.
  const [renamingSpeakerAt, setRenamingSpeakerAt] = useState<string | null>(
    null,
  );
  /// Which section is being improved, or null. One at a time: the two calls
  /// would each read the meeting row and write a version from it, and the second
  /// to land would carry a copy of the first section from before the first
  /// finished.
  const [improving, setImproving] = useState<
    "key_points" | "action_items" | null
  >(null);
  /// Bumped whenever the version history changes, so an open panel refetches.
  const [versionsKey, setVersionsKey] = useState(0);
  const confirmBeforeRecordingRef = useRef(settings.confirm_before_recording);
  confirmBeforeRecordingRef.current = settings.confirm_before_recording;
  const scrollRef = useRef<HTMLDivElement>(null);
  /// Whether the reading column is following the newest line. A ref, not state:
  /// it is written from a scroll handler at up to one frame per pixel, and this
  /// component owns the meetings, the transcript and the recorder status — a
  /// state write here would re-render the whole shell per frame.
  const pinnedRef = useRef(true);
  /// Where the titlebar was pressed, until the pointer moves far enough for it
  /// to be a drag rather than a click.
  const pressRef = useRef<{ x: number; y: number; started: boolean } | null>(
    null,
  );
  /// Open by default: the list of meetings is the reason the window is this
  /// wide. Not persisted — hiding it is a gesture for the current task, not a
  /// preference, and a window that came back without its list would read as
  /// having lost it.
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showOnboarding, setShowOnboarding] = useState(
    !initialSettings.onboarding_complete,
  );

  const selected = useMemo(
    () => meetings.find((m) => m.id === selectedId) ?? null,
    [meetings, selectedId],
  );

  /// Ask where this meeting's recording is, whenever which meeting or what it is
  /// doing changes. The status is a dependency and not decoration: the backend
  /// refuses to name the file of a recording in progress, so a meeting asked
  /// about while it was still being captured has to be asked again once it is
  /// not.
  ///
  /// The late answer to an abandoned request is dropped. Selecting two meetings
  /// quickly would otherwise leave the first one's recording under the second
  /// one's transcript, which is the one mistake a player like this must not make.
  const selectedStatus = selected?.status;
  useEffect(() => {
    if (!selectedId) {
      setAudioPath(null);
      return;
    }
    let live = true;
    setAudioPath(undefined);
    api
      .meetingAudioPath(selectedId)
      .then((p) => {
        if (live) setAudioPath(p);
      })
      .catch(() => {
        if (live) setAudioPath(null);
      });
    return () => {
      live = false;
    };
  }, [selectedId, selectedStatus]);

  /// Put the playhead where a line was said, and start playing only when the
  /// thing that was clicked says it will.
  ///
  /// Every segment moves the playhead — that is what makes the transcript an
  /// index into the recording — but only the offset above a turn, which reads
  /// "play from here", also starts the audio. The bubbles cannot: their click
  /// already opens the correction, and a line that begins playing under the
  /// typing would be a second thing that click did.
  ///
  /// Best effort, deliberately: a click landing before the file's metadata has
  /// arrived has nowhere to seek to, and the honest response is to do nothing
  /// rather than to queue a jump the user has stopped expecting.
  const seekTo = useCallback((ms: number, play: boolean) => {
    const el = audioRef.current;
    if (!el) return;
    el.currentTime = ms / 1000;
    if (play) void el.play().catch(() => {});
  }, []);

  /// The phase the header narrates, or `null` when there is nothing to say.
  /// `ready` is the end of the walk and clears the state; the two failures are
  /// outcomes, and each belongs beside the thing it happened to rather than
  /// under a spinner that has stopped spinning.
  const working =
    progress &&
    progress.phase !== "ready" &&
    progress.phase !== "summary_failed" &&
    progress.phase !== "final_pass_failed"
      ? progress.phase
      : null;

  /// Matched on the meeting, not just the phase: `meeting://progress` reports
  /// the recording that just stopped, which is not necessarily the one on
  /// screen by the time the user reads this.
  const summaryFailed =
    progress?.phase === "summary_failed" && progress.meeting_id === selectedId;

  const finalPassFailed = finalPassFailedId === selectedId;

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

  /// Which load is the current one. Two of these can be in flight — a click on
  /// the sidebar and the reload that follows a recording finishing — and the
  /// slower request resolving last would paint its transcript under the other
  /// one's title.
  const loadRequest = useRef(0);

  const loadMeeting = useCallback(async (id: string) => {
    const mine = ++loadRequest.current;
    setSelectedId(id);
    try {
      const [tr, m] = await Promise.all([
        api.getTranscript(id),
        api.getMeeting(id),
      ]);
      // Somebody asked for a different meeting while this was in flight, and
      // they asked more recently. This answer is about the previous one.
      if (loadRequest.current !== mine) return;
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

  /// Ask the model to improve one section, keeping what it replaces.
  ///
  /// A refusal is surfaced rather than swallowed: the command rejects when the
  /// model answers with prose instead of a list, and that is the case where the
  /// meeting deliberately keeps what it had.
  const improve = useCallback(
    async (section: "key_points" | "action_items") => {
      if (!selectedId || improving) return;
      setImproving(section);
      setError(null);
      try {
        await api.refineSummarySection(selectedId, section);
        setVersionsKey((k) => k + 1);
        await loadMeeting(selectedId);
      } catch (e) {
        setError(String(e));
      } finally {
        setImproving(null);
      }
    },
    [selectedId, improving, loadMeeting],
  );


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

  /// These three sit above the effects that call them, and they are
  /// `useCallback` rather than plain declarations for one reason: an effect
  /// that lists them as dependencies re-registers when they change. A listener
  /// registered once with `[]` kept the `t` of the first render, and
  /// `i18n.tsx:41` builds `t` from a catalog that starts empty — so a gate
  /// failure reached from the keyboard reported the literal `gate.local_stt`
  /// instead of the sentence.
  const handleStart = useCallback(async () => {
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
  }, [t, refreshMeetings]);

  /// The dock and both keyboard paths go through here, so the reminder cannot
  /// be skipped by starting a recording from the keyboard.
  ///
  /// Reads the preference from a ref rather than the closed-over value: the
  /// global-hotkey listener outlives a settings change, so it would otherwise
  /// keep the setting as it was at startup and go on asking after the user
  /// opted out.
  const requestStart = useCallback(() => {
    if (confirmBeforeRecordingRef.current) {
      setConfirmingRecord(true);
      return;
    }
    void handleStart();
  }, [handleStart]);

  const handleStop = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const m = await api.stopRecording();
      setStatus(await api.recorderStatus());
      await refreshMeetings();
      await loadMeeting(m.id);
      // The tab switch moved onto the `summarizing` phase. This call only
      // returns once every step has finished, so switching here landed the user
      // on a finished answer rather than letting them watch it being written.
      await refreshGate();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refreshMeetings, loadMeeting, refreshGate]);

  useEffect(() => {
    (async () => {
      void refreshDevices();
      // Drains on read, so a failure here loses the notice rather than
      // repeating it. Not worth an error banner of its own: the pick has
      // already been moved, and the picker shows what it moved to.
      void api.retiredModels().then(setRetired).catch(() => {});
      // Whether the OS granted the accelerator is not a startup failure: the
      // in-app listener works regardless, so a rejection here leaves the note
      // off rather than putting an error banner on the first screen.
      setShortcut(await api.shortcutStatus().catch(() => null));
      // Asked once, at launch, and only here: `recording` and `paused` are
      // states nothing but an interrupted run leaves behind, so asking again
      // later would be asking about a recording that is under way.
      //
      // A failure loses the offer for this launch rather than raising a banner.
      // The rows are untouched, so the next start of the app asks again.
      void api.interruptedMeetings().then(setInterrupted).catch(() => {});
      try {
        await refreshMeetings();
        setModels(await api.listModels());
        setStatus(await api.recorderStatus());
        await refreshGate();
      } catch (e) {
        setError(String(e));
      }
      try {
        // Not while offline mode is on. An update check is a request to a
        // server that learns this installation exists, which is exactly what
        // the switch is for — and a "fully offline" mode that phones home
        // about versions would be the kind of small lie that makes the rest of
        // the promise unbelievable.
        if (settings.offline_mode) return;
        // Nor when the user has turned the check itself off, which is the
        // narrower version of the same wish: no launch-time request, and no
        // installer pulled behind it. It governs what happens on its own and
        // nothing else — an update already found and offered can still be
        // installed by clicking, because that is the user asking.
        if (!settings.auto_update_check) return;
        // Offered, never applied on its own: installing relaunches the app, and
        // relaunching can throw away a recording in progress. Deciding that for
        // someone is not ours to do.
        //
        // The bytes, though, are fetched now. Downloading on the click meant
        // the user decided to update and then waited on a progress bar that
        // does not exist; doing it here makes the button instant. Failure is
        // silent on purpose — the update is still offered, and the click falls
        // back to downloading then.
        const update = await check();
        if (!update) return;
        setPendingUpdate(update);
        try {
          await update.download();
          setUpdateReady(true);
        } catch {
          /* offered anyway; the click will fetch it */
        }
      } catch {
        /* no release endpoint yet */
      }
    })();
  }, [refreshMeetings, refreshGate, refreshDevices]);

  useEffect(() => {
    /// `listen()` is a promise, and the cleanup below runs synchronously — so
    /// it used to run against an empty array while the subscriptions were still
    /// resolving. Under `React.StrictMode` (main.tsx) the mount→unmount→mount
    /// cycle therefore left the first set attached forever and every event
    /// fired twice in dev, which would have double-toggled the recorder the
    /// moment the shortcut started working. `track` closes that: a
    /// subscription that lands after teardown unsubscribes itself.
    let live = true;
    const unsubs: Array<() => void> = [];
    const track = (un: () => void) => {
      if (live) unsubs.push(un);
      else un();
    };
    (async () => {
      track(
        await listen<LiveTranscript>("transcript://append", (e) => {
          setTranscript(e.payload);
          // A window that filled clears whatever the last failure said, so a
          // provider that recovers stops shouting without needing a second
          // signal.
          setLiveSttError(null);
        }),
      );
      track(
        // Never swallowed. A cloud provider rejecting every chunk — wrong model
        // id, no credit, a 400 — used to be an empty screen and no explanation.
        await listen<string>("transcript://error", (e) => {
          setLiveSttError(e.payload);
        }),
      );
      track(
        await listen<boolean>("recording://silent", (e) => setSilent(e.payload)),
      );
      track(
        await listen<Deaf>("recording://deaf", (e) => setDeaf(e.payload)),
      );
      track(
        // The window stops it, through the same command a click goes through.
        // Stopping from the ticker would be a second stop path with none of the
        // guards that one has, and the two would drift.
        await listen("recording://stop-silent", () => {
          setSilent(false);
          void handleStop();
        }),
      );
      track(
        await listen<MeetingRecord>("meeting://ready", (e) => {
          setMeetings((prev) => {
            const rest = prev.filter((m) => m.id !== e.payload.id);
            return [e.payload, ...rest];
          });
          // The transcript comes with the selection. This handler already
          // moved the selection to the finished meeting, and a stop pressed
          // on the floating card goes straight to the backend — so nothing
          // else was reloading the pane, and it went on showing whichever
          // transcript happened to be up. What the backend last wrote is now
          // the re-read of the whole recording, which makes the difference
          // the length of the meeting rather than the last utterance of it.
          void loadMeeting(e.payload.id);
        }),
      );
      track(
        await listen<MeetingProgress>("meeting://progress", (e) => {
          const p = e.payload;
          // `ready` is the end of the walk, not a step in it: the meeting is
          // on screen by then and a spinner beside it would be a lie.
          setProgress(p.phase === "ready" ? null : p);
          // `saving` is the first phase of a walk, so it is where a previous
          // meeting's note is cleared — the note outlives its own phase and
          // would otherwise still be there for the next recording.
          if (p.phase === "saving") setFinalPassFailedId(null);
          if (p.phase === "final_pass_failed") {
            setFinalPassFailedId(p.meeting_id);
          }
          // The sidebar already paints a dot for any status other than
          // `ready`, so re-reading the list here lights it for free.
          if (p.phase === "transcribing") void refreshMeetings();
          // Here rather than in `handleStop`: the point is to watch the pane
          // the answer lands in while it is being written, instead of being
          // teleported to it once it is already done.
          if (p.phase === "summarizing") setTab("summary");
        }),
      );
      track(
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
    return () => {
      live = false;
      unsubs.forEach((u) => u());
    };
  }, [handleStop, requestStart, refreshMeetings, loadMeeting]);

  /// The in-app half of the record shortcut, and the half that actually works.
  ///
  /// A webview `keydown` needs no grant from the OS — the global registration
  /// does, and on a machine where another app already owns the combination it
  /// is refused. This covers the focused case, which is exactly the case the
  /// empty state's `<kbd>` describes. It does not replace the global shortcut:
  /// unfocused is the case that matters most for a meeting recorder, since you
  /// are in the call, not in Vesper.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      // The combination the backend actually registered, not a literal. It is a
      // setting now, and a hardcoded Ctrl+Shift+R here would keep working
      // alongside whatever the user picked — two keys that both start a
      // recording, one of which they thought they had replaced.
      if (!matchesAccelerator(e, shortcut?.accelerator)) return;
      // Bubble phase and a tag guard, so a field the user is typing in wins.
      // The capture phase would be right for a shortcut that must fire
      // unconditionally; this is not one.
      const el = e.target as HTMLElement | null;
      if (el?.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el?.tagName ?? "")) {
        return;
      }
      // A dialog or the drawer is a question waiting for an answer. Starting a
      // recording underneath one is not an answer.
      if (confirmingRecord || pendingDelete || showSettings || showOnboarding) return;
      // Not optional: Ctrl+Shift+R is WebView2's hard reload. Without it the
      // dev build reloads the page and loses the recording in flight — and
      // release builds disable the accelerator, so it looks correct in
      // `tauri build` and misbehaves in `tauri dev`.
      e.preventDefault();
      // Auto-repeat fires this dozens of times a second while the key is down,
      // and `busy` covers the slower version of the same thing: a second press
      // while the first start or stop is still in flight. Either one starts a
      // recording and immediately stops it, with an error banner from whichever
      // request lost.
      if (e.repeat || busy) return;
      if (status?.recording) void handleStop();
      else requestStart();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    shortcut?.accelerator,
    status?.recording,
    busy,
    confirmingRecord,
    pendingDelete,
    showSettings,
    showOnboarding,
    handleStop,
    requestStart,
  ]);

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

  // Levels and the clock, ten times a second. They used to share the transcript's
  // 1200ms tick, which is why the meters read as static even once their scale was
  // fixed: a level that refreshes once a second is a still picture. This call only
  // reads atomics on the other side, so it is cheap enough to ask this often — and
  // it is the one thing that must keep moving while a slow transcription runs.
  useEffect(() => {
    if (!status?.recording || status.paused) return;
    const id = window.setInterval(async () => {
      try {
        setStatus(await api.recorderStatus());
      } catch {
        /* keep UI alive */
      }
    }, 100);
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

  /// A rename belongs to the meeting it was opened on. Clicking away commits it
  /// through the field's own blur, but nothing else does — the tray, a
  /// `meeting://ready` landing, the sidebar's arrow keys — and an open field
  /// carrying the previous meeting's name is how the wrong title gets saved.
  useEffect(() => {
    setRenaming(false);
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
    setSummarizingId(selectedId);
    // Before the await, not after it. The pane that is about to be written is
    // the one worth watching while it is written — switching to it once the
    // answer is already there is a teleport, and it left the working state
    // rendering on a tab nobody was looking at. The pipeline after a recording
    // has always done it this way; the button was the odd one out.
    setTab("summary");
    try {
      await api.summarize(selectedId, template);
      await loadMeeting(selectedId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setSummarizingId(null);
    }
  }

  async function handleImport() {
    // The card must not float over the chooser this is about to open. Both this
    // and the export below are Vesper's own modals, and from the backend a
    // modal and another application look identical.
    await api.setModalOpen(true);
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
      await api.setModalOpen(false);
      setBusy(false);
    }
  }

  async function handleExport(format: "md" | "pdf" | "docx") {
    if (!selectedId) return;
    await api.setModalOpen(true);
    try {
      const path = await save({
        // The meeting's own name, sanitised by the backend, so a folder of
        // exports is readable instead of `vesper-export (4).md`. One extra IPC
        // hop on a click that is about to open a native dialog — free — and it
        // keeps the Windows filename rules where `cargo test` can reach them.
        defaultPath: await api
          .suggestedExportName(selectedId, format)
          .catch(() => `vesper-export.${format}`),
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!path) return;
      await api.exportMeeting(selectedId, path, format);
    } catch (e) {
      setError(String(e));
    } finally {
      await api.setModalOpen(false);
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

  /// Nothing is written optimistically: the title on screen only changes once
  /// the backend has kept it, so a rejected rename leaves the header showing
  /// what the database actually holds and there is nothing to roll back.
  ///
  /// The id is a parameter rather than read from `selected`, so a rename that
  /// commits on the way out of the field can never land on the meeting the user
  /// just clicked.
  async function commitRename(id: string, next: string) {
    setRenaming(false);
    const title = next.trim();
    if (!title || title === meetings.find((m) => m.id === id)?.title) return;
    try {
      const m = await api.renameMeeting(id, title);
      setMeetings((prev) => prev.map((x) => (x.id === m.id ? m : x)));
    } catch (e) {
      setError(String(e));
    }
  }

  /// The name this meeting stored for a channel, or "" when it stored none.
  ///
  /// What the rename field is filled with. Never the fallback: a field
  /// pre-filled with "Me" that somebody opens and walks away from would store
  /// the English word, and freeze the meeting in a language they may not read.
  function ownSpeakerName(speaker: Speaker) {
    return (
      (speaker === "me" ? selected?.speaker_me : selected?.speaker_others) ?? ""
    );
  }

  /// What this meeting calls a channel: its own name if it has one, otherwise
  /// the app's own word in the user's language. The fallback is computed here
  /// and never sent back — a meeting nobody renamed has to follow the language
  /// they pick next.
  function speakerName(speaker: Speaker) {
    return (
      ownSpeakerName(speaker) ||
      t(speaker === "me" ? "speaker.me" : "speaker.others")
    );
  }

  /// Nothing is written optimistically, for the reason the title rename is not:
  /// the backend cleans what it is given — the name reaches a model prompt and
  /// an exported file — so what comes back is what is true, and a refusal leaves
  /// the header showing what the database holds.
  ///
  /// One channel, never the pair. Sending both would send this snapshot's idea
  /// of the other one too, and renaming the second while the first is still in
  /// flight would carry that stale value back over a rename that had already
  /// succeeded.
  async function commitSpeakerRename(
    meeting: MeetingRecord,
    speaker: Speaker,
    next: string,
  ) {
    setRenamingSpeakerAt(null);
    // Blank clears. `null` is the absence of a name, which is what puts the
    // channel back to the app's own word — and it is what typing nothing means.
    const name = next.trim() || null;
    const was =
      (speaker === "me" ? meeting.speaker_me : meeting.speaker_others) ?? null;
    if (name === was) return;
    try {
      const m = await api.setSpeakerName(meeting.id, speaker, name);
      setMeetings((prev) => prev.map((x) => (x.id === m.id ? m : x)));
    } catch (e) {
      setError(String(e));
    }
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

  /// Take the answered offer off the queue, whichever way it was answered.
  function dismissRecovery() {
    setInterrupted((queue) => queue.slice(1));
  }

  /// Finish a meeting whose recording was cut short.
  ///
  /// The offer comes down first: this reads the whole recording, which on a long
  /// meeting is minutes, and a dialog left over it would sit there through all
  /// of them. The `meeting://progress` events are what narrate the wait, the
  /// same ones a stop raises.
  async function handleRecover(id: string) {
    dismissRecovery();
    setBusy(true);
    setError(null);
    try {
      await api.recoverMeeting(id);
      await refreshMeetings();
      await loadMeeting(id);
    } catch (e) {
      // The meeting is untouched by a recovery that failed, so it is offered
      // again the next time the app starts. Nothing here has to put it back.
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleDiscardInterrupted(id: string) {
    dismissRecovery();
    try {
      await api.discardInterruptedMeeting(id);
      await refreshMeetings();
    } catch (e) {
      setError(String(e));
    }
  }

  /// Picking a capture device in the dock writes straight through — there is no
  /// Save button within 400px of it, and the choice is one click away from the
  /// recording it governs. Same path as every other save, so it fails the same
  /// way, and the gate is re-read because "no microphone" is one of the reasons
  /// it blocks — and now "both channels off" is another.
  ///
  /// The device id travels with the switch rather than instead of it: turning a
  /// channel off keeps the device it was pointing at, so it comes back to that
  /// one and not to the system default.
  async function handlePickDevice(
    kind: "mic" | "system",
    id: string | null,
    enabled: boolean,
  ) {
    try {
      const next = await api.saveSettings(
        kind === "mic"
          ? { ...settings, mic_device_id: id, capture_me: enabled }
          : { ...settings, system_device_id: id, capture_others: enabled },
      );
      setSettings(next);
      onSettingsChange(next);
      await refreshGate();
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

          Dragging is handled here rather than by `data-tauri-drag-region`,
          for two reasons the attribute cannot cover.

          Its handler tests `event.target`, and the three grid columns below
          are block elements that cover the whole bar — so every press landed
          on a column, never on the header, and the only draggable pixels in
          the window were the 16px gaps between them. The columns that hold
          nothing to click are `pointer-events-none` now, which is what lets a
          press reach this element at all.

          And the window opens maximised. Tauri's drag region calls
          `startDragging` and nothing else, which a maximised window ignores;
          Windows restores the window and takes it with the cursor. That takes
          an `unmaximize` first, which means owning the press.

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
        onPointerDown={(e) => {
          // Left button only, and only on the bar itself — a press that landed
          // on a control is that control's.
          if (e.button !== 0 || e.target !== e.currentTarget) return;
          // Captured, so the release comes back here even if the pointer has
          // left the bar by then. Without it a press that ended over the
          // content below left the press recorded, and the next ordinary move
          // across the titlebar started a drag with no button held.
          e.currentTarget.setPointerCapture(e.pointerId);
          pressRef.current = { x: e.clientX, y: e.clientY, started: false };
        }}
        onPointerMove={(e) => {
          const press = pressRef.current;
          if (!press || press.started) return;
          // Four pixels, because a press that never moves is a click. Starting
          // the drag on pointerdown enters the OS move loop immediately, and
          // that loop swallows the second click of a double-click — which is
          // how maximise-by-double-click went missing.
          if (Math.abs(e.clientX - press.x) < 4 && Math.abs(e.clientY - press.y) < 4) {
            return;
          }
          press.started = true;
          // The identity of the press is the cancellation token: a release
          // clears the ref, and the two awaits below check it before acting.
          void dragWindow(() => pressRef.current === press);
        }}
        onPointerUp={() => {
          pressRef.current = null;
        }}
        onPointerCancel={() => {
          pressRef.current = null;
        }}
        onLostPointerCapture={() => {
          pressRef.current = null;
        }}
        onDoubleClick={(e) => {
          if (e.target !== e.currentTarget) return;
          void getCurrentWindow().toggleMaximize();
        }}
        // `pr-0` now: the window controls run to the window's own edge, the way
        // every other application on the platform draws them. The 46px targets
        // supply their own inset.
        className="grid h-12 shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 pl-4 pr-0"
      >
        {/* `pointer-events-none` on the block, and the toggle opts back in:
            nothing else here is clickable, and while the whole block ate
            presses the left half of the titlebar could not be dragged. */}
        <div className="pointer-events-none flex items-center gap-2 [&>button]:pointer-events-auto">
          <Button
            variant="ghost"
            size="icon"
            data-testid="btn-toggle-sidebar"
            onClick={() => setSidebarOpen((v) => !v)}
            title={t(sidebarOpen ? "sidebar.hide" : "sidebar.show")}
            aria-label={t(sidebarOpen ? "sidebar.hide" : "sidebar.show")}
            aria-expanded={sidebarOpen}
          >
            {sidebarOpen ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />}
          </Button>
          {/* Only with the sidebar closed and a meeting open. The way back to
              the empty screen lived in the sidebar and nowhere else, so closing
              the sidebar took it away — and closing the sidebar is exactly when
              somebody is reading a meeting and wants out of it.

              Hidden with the sidebar open on purpose: the same action is right
              there in the list header, and two buttons for one thing on screen
              at once is how a user learns to distrust both. */}
          {!sidebarOpen && selected && (
            <Button
              variant="ghost"
              size="icon"
              onClick={startNewMeeting}
              title={t("nav.new")}
              aria-label={t("nav.new")}
            >
              <House size={16} />
            </Button>
          )}
          {/* No radius: the asset is the mark alone now, not a rounded tile, so
              a corner clip would shave the artwork instead of a background. */}
          <img src={logo} alt="" className="h-8 w-8" />
          {/* Flat, not a gradient: the gradient made the brightest, most
              saturated thing on screen a piece of chrome the user cannot act
              on, and it was the only reason --color-accent-2 existed. */}
          <span className="text-lg font-semibold text-fg">{t("app.name")}</span>
        </div>

        {/* Centre column: the transport for a recording in progress, and then
            the phase of the work that follows it. It used to be a badge that
            only said "Recording" while Stop lived in the dock — which is on the
            empty screen, and starting a recording leaves that screen. The
            controls follow the state they control.

            Both in the same `AnimatePresence` with the same `fadeRise`, so Stop
            hands the slot from one to the other in a single beat rather than
            emptying the header for the several seconds the work takes. */}
        {/* Transparent to presses, and its contents are not: the column is
            empty except while recording, and an empty column swallowing the
            middle of the titlebar is the same bug as the identity block. */}
        {/* No `[&>*]` opt-in here, unlike the actions column. Its children are
            readouts wrapped around a couple of buttons, and making the wrapper
            answer presses only moves the problem up a level: the wrapper
            becomes the target and the header still refuses it. The transport
            re-enables its own buttons instead, which works through an
            ancestor that does not answer. */}
        <div className="pointer-events-none flex items-center justify-center">
          <AnimatePresence>
            {status?.recording ? (
              <motion.div key="transport" {...fadeRise}>
                <RecordTransport
                  status={status}
                  busy={busy}
                  onStop={handleStop}
                  onPauseResume={handlePauseResume}
                />
              </motion.div>
            ) : working ? (
              <motion.div key="processing" {...fadeRise}>
                <ProcessingStatus phase={working} />
              </motion.div>
            ) : null}
          </AnimatePresence>
        </div>

        {/* Same treatment as the other two columns, and for the same reason:
            this one is a full grid track with three controls at its end, so
            every pixel to their left was a place the window could not be
            dragged from. The buttons opt back in. */}
        <div className="pointer-events-none flex items-center justify-end gap-1 [&>*]:pointer-events-auto">
          {/* Renders only while something is actually leaving the machine, which
              is what makes it worth reading when it appears. */}
          <EgressBadge
            settings={settings}
            onOpenSettings={() => setShowSettings(true)}
          />
          {/* The theme is one click from anywhere, not four (gear → Appearance →
              pick → close). It applies immediately and writes straight through
              to settings, because there is no draft out here to be dirty and
              nothing to save. `theme` is read off the row rather than off the
              DOM so the button and the stored value cannot disagree. */}
          <Button
            variant="ghost"
            size="icon"
            onClick={() => {
              const previous = settings.theme;
              const next = previous === "dark" ? "light" : "dark";
              applyTheme(next);
              setSettings((s) => ({ ...s, theme: next }));
              // Only the theme goes over the wire. Sending a whole snapshot for
              // one field made this a lost update: a toggle still in flight
              // landed after a drawer Save carrying the pre-drawer value of
              // every other setting.
              api
                .setTheme(next)
                .then(onSettingsChange)
                .catch((e) => {
                  // Put it back. Leaving the header showing a theme the database
                  // does not have means the next launch silently disagrees with
                  // what is on screen.
                  applyTheme(previous === "dark" ? "dark" : "light");
                  setSettings((s) => ({ ...s, theme: previous }));
                  setError(String(e));
                });
            }}
            title={t(settings.theme === "dark" ? "theme.light" : "theme.dark")}
            aria-label={t(settings.theme === "dark" ? "theme.light" : "theme.dark")}
          >
            {settings.theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setShowSettings(true)}
            title={t("nav.settings")}
            aria-label={t("nav.settings")}
          >
            <Settings size={16} />
          </Button>
          {/* Drawn by the app because `decorations` is off. The system frame put
              a light grey Windows bar above a cream window with no way to theme
              it, which is the seam the product owner is pointing at. */}
          <WindowControls />
        </div>
      </header>

      {/* The sidebar is the backdrop, not a panel that folds. It keeps its
          place and its size, and the card slides across it — which is why the
          animation belongs to the card and there is nothing here that squeezes
          a list of meetings into a narrower box while the user reads it. */}
      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        {/* `inert` while covered. The card hides it from the eye, and without
            this the search field, the import button and every meeting row stay
            in the tab order — reachable, actionable, and with the focus ring
            drawn behind the card where nobody can see it. */}
        <div className="absolute inset-y-0 left-0 w-72" inert={!sidebarOpen}>
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
        </div>

        <motion.main
          // `initial={false}` so the first paint is wherever the sidebar
          // already is, rather than an animation nobody asked for on launch.
          initial={false}
          animate={{ marginLeft: sidebarOpen ? SIDEBAR_WIDTH : 0 }}
          transition={transition.base}
          className="relative z-10 flex min-w-0 flex-1 flex-col">
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
            {/* Transcription is failing while the recording continues. Not
                dismissible and not an `error`: the audio is still being captured
                and saved, so this is a degraded state rather than a lost one, and
                it goes away by itself when a chunk finally lands. */}
            {liveSttError && status?.recording && (
              <div
                role="status"
                data-testid="live-stt-error"
                className="flex items-start gap-2 border-b border-warn/30 bg-warn/10 px-6 py-2 text-sm text-warn"
              >
                <TriangleAlert size={16} aria-hidden className="mt-0.5 shrink-0" />
                <span>
                  {t("live.stt_failing")}{" "}
                  <span className="text-fg-muted">{liveSttError}</span>
                </span>
              </div>
            )}
            {/* A question, not a warning: the answer is a click OR simply
                speaking again, and the copy says both. It sits with the other
                banners rather than as a modal — a dialog over a running meeting
                is a thing to dismiss before you can see the transcript, and the
                whole point is that the user may not be at the machine. */}
            {/* Which model writes a user's summaries is not something to
                change under them in a log file. Dismissible, and it does not
                come back: the command that fed it drained the list. */}
            {retired.length > 0 && (
              <div
                role="status"
                className="flex flex-wrap items-center gap-3 border-b border-warn/30 bg-warn/10 px-6 py-2 text-sm text-warn"
              >
                <span className="text-fg-muted">
                  {retired
                    .map(([from, to]) =>
                      t("model.retired").replace("{from}", from).replace("{to}", to),
                    )
                    .join(" ")}
                </span>
                <Button
                  size="xs"
                  variant="secondary"
                  className="ml-auto"
                  onClick={() => setRetired([])}
                >
                  {t("action.dismiss")}
                </Button>
              </div>
            )}
            {/* Danger rather than warning, and above the quiet-room notice:
                that one is a question about the people, this is the recording
                not happening. It stays until the input starts working or the
                recording ends — there is nothing to dismiss, because dismissing
                it would not make the audio arrive. */}
            {(deaf.me || deaf.others) && status?.recording && (
              <div
                role="alert"
                data-testid="deaf-notice"
                className="flex flex-wrap items-center gap-3 border-b border-danger/30 bg-danger/10 px-6 py-2 text-sm text-danger"
              >
                <span className="font-medium">{t("deaf.title")}</span>
                <span className="text-fg-muted">
                  {deaf.me && deaf.others
                    ? t("deaf.both")
                    : deaf.me
                      ? t("deaf.me")
                      : t("deaf.others")}
                </span>
                <Button
                  size="xs"
                  variant="secondary"
                  className="ml-auto"
                  onClick={() => setShowSettings(true)}
                >
                  {t("deaf.devices")}
                </Button>
              </div>
            )}
            {silent && status?.recording && (
              <div
                role="status"
                data-testid="silence-notice"
                className="flex flex-wrap items-center gap-3 border-b border-warn/30 bg-warn/10 px-6 py-2 text-sm text-warn"
              >
                <span className="font-medium">{t("silence.title")}</span>
                <span className="text-fg-muted">{t("silence.body")}</span>
                <Button
                  size="xs"
                  variant="secondary"
                  className="ml-auto"
                  onClick={() => {
                    setSilent(false);
                    void api.keepRecording();
                  }}
                >
                  {t("silence.keep")}
                </Button>
              </div>
            )}
            {pendingUpdate && !settings.offline_mode && (
              <div className="flex flex-wrap items-center gap-3 border-b border-accent/30 bg-accent/10 px-6 py-2 text-sm text-accent">
                <span>{t("update.available").replace("{version}", pendingUpdate.version)}</span>
                <Button
                  size="xs"
                  data-testid="update-install"
                  // Installing relaunches the app, and a relaunch during a
                  // recording loses the audio that has not been written yet.
                  // The offer stays on screen; it just cannot be taken until
                  // the recording is over.
                  disabled={status?.recording}
                  title={status?.recording ? t("update.busy") : undefined}
                  onClick={async () => {
                    const update = pendingUpdate;
                    setPendingUpdate(null);
                    try {
                      setUpdateNote(t("update.installing"));
                      // `install` alone when the bytes are already here, which
                      // is the usual case — `downloadAndInstall` would fetch
                      // them a second time.
                      //
                      // And never fetch them at all while offline mode is on.
                      // An update discovered before the switch was thrown
                      // leaves this offer behind it, and accepting it would
                      // reach the network on a click the user thinks is local.
                      // Installing bytes already on disk is not egress and
                      // stays allowed.
                      if (updateReady) await update.install();
                      else if (settings.offline_mode)
                        throw new Error(t("update.offline"));
                      else await update.downloadAndInstall();
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
                  {updateReady ? t("update.restart") : t("update.install")}
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
                {/* The rule spans the card; the content sits on the column.
                    Binding the rule to `max-w-pane` as well left it starting and
                    stopping a couple of hundred pixels short of the card on
                    either side — a hairline floating inside a panel, aligned to
                    nothing, above a title indented from an edge that was still
                    visible behind it. Chrome spans, prose is measured, which is
                    how the window header and the sidebar already work. */}
                <div className="border-b border-border">
                  <div className="mx-auto flex w-full max-w-pane items-center justify-between px-6 py-3">
                    <div className="min-w-0">
                      {/* Click to edit, in place. `bg-transparent` and the same
                          size and weight as the heading it replaces, so the title
                          does not read as a form field at rest — the field is the
                          heading, not a control beside it. */}
                      {renaming ? (
                        <input
                          autoFocus
                          data-testid="rename-input"
                          aria-label={t("nav.rename")}
                          defaultValue={selected.title}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              void commitRename(selected.id, e.currentTarget.value);
                            } else if (e.key === "Escape") {
                              e.preventDefault();
                              // Put the original back before closing. A browser
                              // that fires blur on the way out would otherwise
                              // commit the abandoned edit; with the value
                              // restored, that commit is a no-op.
                              e.currentTarget.value = selected.title;
                              setRenaming(false);
                            }
                          }}
                          onBlur={(e) => void commitRename(selected.id, e.currentTarget.value)}
                          className={`w-full rounded-sm bg-transparent text-base font-medium ${FOCUS}`}
                        />
                      ) : (
                        // The control is inside the heading rather than being
                        // the heading: an `<h1 onClick>` is a click target with
                        // no keyboard route, and every other affordance in this
                        // app has one. `title` also gives the full name back on
                        // hover once the heading truncates.
                        <h1 className="truncate text-base font-medium">
                          <button
                            type="button"
                            onClick={() => setRenaming(true)}
                            title={selected.title}
                            aria-label={t("nav.rename")}
                            className={`max-w-full cursor-text truncate rounded-sm text-left ${FOCUS}`}
                          >
                            {selected.title}
                          </button>
                        </h1>
                      )}
                      {/* When it happened, in the reader's own time, and how long
                          it ran. The status word used to sit here and said
                          nothing in the normal case; the sidebar's dot already
                          covers the abnormal one. */}
                      <p className="text-xs tabular-nums text-fg-muted">
                        {formatMeetingDateTime(selected.created_at)} ·{" "}
                        {formatDuration(selected.duration_ms)}
                        {/* Only when a paid provider was actually called. A
                            meeting transcribed and summarised on this machine
                            has no price, and printing "$0.00" for it would be a
                            claim about spending rather than the absence of any. */}
                        {selected.cost_label ? (
                          <>
                            {" · "}
                            <span title={t("meeting.cost")}>
                              {selected.cost_label}
                            </span>
                          </>
                        ) : null}
                      </p>
                    </div>
                    {/* Five siblings used to read as five equal actions. Summarize
                        is the one primary; MD/PDF/DOCX are one action with a format
                        parameter, so they sit inside a single bordered group.
                        The inset is horizontal only: a `p-1` shell would stand 34px
                        tall between two 24px chips, and a group that is half again
                        the height of its neighbours reads as a different tier of
                        control rather than a bracket around three of them. */}
                    {/* `shrink-0` beside the title's `min-w-0`: a generated or
                        typed name can be arbitrarily long, and without the pair
                        the flex algorithm resolves the overflow by squeezing the
                        buttons instead of truncating the heading. */}
                    <div className="flex shrink-0 items-center gap-2">
                      <SummarizeButton
                        disabled={busy}
                        onSummarize={handleSummarize}
                      />
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
                    </div>
                  </div>
                  {/* Under the title rather than in its own strip: this is the
                      meeting's recording, not a second region, and a rule of its
                      own directly beneath the header's would read as one.
                      Nothing is drawn while the answer is in flight — see
                      `audioPath` — and nothing is drawn for the meeting being
                      recorded either, whose file the backend refuses to name
                      while it is still growing. Saying "not on this computer"
                      about audio that is arriving is worse than saying nothing. */}
                  {audioPath !== undefined &&
                    selected.status !== "recording" &&
                    selected.status !== "paused" && (
                      <div className="mx-auto w-full max-w-pane px-6 pb-3">
                        {audioPath ? (
                          <AudioPlayer
                            // Remounted per recording, so the transport does not
                            // open the next meeting showing the previous one's
                            // position.
                            key={audioPath}
                            audioRef={audioRef}
                            path={audioPath}
                            durationMs={selected.duration_ms}
                            onUnavailable={() => setAudioPath(null)}
                          />
                        ) : (
                          <p className="text-xs text-fg-muted">
                            {t("player.unavailable")}
                          </p>
                        )}
                      </div>
                    )}
                </div>

                {/* The copy control rides the tab rule rather than the pane:
                    the transcript scrolls and a button inside it would leave
                    with the rows. `relative` + absolute keeps `Tabs` owning its
                    own width, which its roving keyboard nav measures. */}
                <div className="relative border-b border-border">
                  <Tabs
                    idPrefix="content"
                    className="mx-auto w-full max-w-pane px-6"
                    value={tab}
                    onChange={setTab}
                    items={[
                      { id: "transcript", label: t("tab.transcript") },
                      { id: "summary", label: t("tab.summary") },
                      { id: "notes", label: t("tab.notes") },
                      { id: "chat", label: t("tab.chat") },
                    ]}
                  />
                  {tab === "transcript" && transcript.segments?.length ? (
                    <div className="pointer-events-none absolute inset-y-0 right-0 mx-auto flex w-full max-w-pane items-center justify-end px-6">
                      <CopyButton
                        className="pointer-events-auto"
                        label={t("tab.transcript")}
                        text={transcript.segments
                          .map(
                            (s) =>
                              `[${formatDuration(s.start_ms)}] ${speakerName(
                                s.speaker,
                              )}: ${s.text}`,
                          )
                          .join("\n")}
                      />
                    </div>
                  ) : null}
                </div>

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
                  // Chat owns its own scrolling: the composer has to stay at
                  // the foot of the pane, and a scroller wrapping a scroller
                  // puts it wherever the conversation happens to end. The other
                  // two tabs are documents and scroll as one.
                  // The document padding is for the documents. Chat carries its
                  // own — a scroller with `px-4 py-5` and a composer with
                  // `pb-4` — and taking this pane's `px-6 pb-6` on top of it
                  // floated the composer 40px above the foot of the card.
                  className={`min-h-0 flex-1 ${
                    tab === "chat"
                      ? "flex flex-col overflow-hidden"
                      : "overflow-y-auto px-6 pb-6 pt-4"
                  }`}
                >
                  <AnimatePresence mode="wait">
                    {tab === "transcript" && (
                      <motion.div
                        key="transcript"
                        ref={pinOnPaneMount}
                        {...fadeRise}
                        // `max-w-pane` (1024px), not `max-w-reading`: the point
                        // of two sides is the split, and a 576px column gives it
                        // nowhere to happen. Each bubble caps at `max-w-reading`
                        // instead, so the measure is protected while the sides
                        // are genuinely separated.
                        className="mx-auto max-w-pane"
                        data-testid="transcript-panel"
                      >
                        {/* Above the lines, because it says what the lines are:
                            these came from the live pass and the re-read that
                            was meant to replace them did not finish. Small and
                            in the pane rather than a banner across the top —
                            the recording, the transcript and the summary are
                            all there, and only the improvement is missing. */}
                        {finalPassFailed && (
                          <p
                            data-testid="final-pass-failed"
                            className="mb-4 text-xs text-fg-muted"
                          >
                            {t("processing.final_pass_failed")}
                          </p>
                        )}
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
                              // This machine's microphone is the user, so it
                              // takes the right side; the computer's audio is
                              // the other people in the meeting and takes the
                              // left. Reversing these makes every transcript
                              // wrong in a way no test can see.
                              const me = s.speaker === "me";
                              return (
                                <motion.div
                                  key={s.id}
                                  {...segmentArrive}
                                  className={clsx(
                                    "flex w-full flex-col",
                                    me ? "items-end" : "items-start",
                                    // 24px at a turn change against 4px within
                                    // one: 6:1 is enough for the grouping to
                                    // read without drawing a rule.
                                    i === 0 ? "" : opens ? "mt-6" : "mt-1",
                                  )}
                                >
                                  {/* One meta line per TURN, not per line — the
                                      one real win of the gutter list this
                                      replaces. Both fields at `text-fg-subtle`:
                                      the timestamp is a seek offset into the
                                      recording, i.e. content, and content
                                      clears 4.5:1. `flex-row-reverse` on the me
                                      side so both read outward-in from their
                                      own edge. */}
                                  {opens && (
                                    <div
                                      className={clsx(
                                        "flex items-baseline gap-2 px-1 pb-1 text-2xs text-fg-subtle",
                                        me && "flex-row-reverse",
                                      )}
                                    >

                                      {/* Click the name to change it, in place
                                          and at the same size, like the title
                                          in the header above. The name belongs
                                          to the meeting, so editing it here
                                          renames every turn — and the export,
                                          the copy and the model's copy with
                                          them. */}
                                      {renamingSpeakerAt === s.id ? (
                                        <input
                                          autoFocus
                                          aria-label={t("speaker.rename")}
                                          // The word the transcript falls back
                                          // to, shown but not stored: clearing
                                          // the field is how a channel goes
                                          // back to it.
                                          placeholder={speakerName(s.speaker)}
                                          defaultValue={ownSpeakerName(s.speaker)}
                                          onKeyDown={(e) => {
                                            if (e.key === "Enter") {
                                              e.preventDefault();
                                              void commitSpeakerRename(
                                                selected,
                                                s.speaker,
                                                e.currentTarget.value,
                                              );
                                            } else if (e.key === "Escape") {
                                              e.preventDefault();
                                              // Put the name back before
                                              // closing, so the blur that
                                              // follows commits nothing.
                                              e.currentTarget.value =
                                                ownSpeakerName(s.speaker);
                                              setRenamingSpeakerAt(null);
                                            }
                                          }}
                                          onBlur={(e) =>
                                            void commitSpeakerRename(
                                              selected,
                                              s.speaker,
                                              e.currentTarget.value,
                                            )
                                          }
                                          className={`w-32 rounded-sm bg-transparent text-2xs font-medium ${FOCUS}`}
                                        />
                                      ) : (
                                        <button
                                          type="button"
                                          onClick={() =>
                                            setRenamingSpeakerAt(s.id)
                                          }
                                          aria-label={t("speaker.rename")}
                                          className={`cursor-text rounded-sm font-medium ${FOCUS}`}
                                        >
                                          {speakerName(s.speaker)}
                                        </button>
                                      )}
                                      {/* The offset was already the seek target
                                          in everything but function, so it is
                                          the one control that starts playing.
                                          The bubbles below only move the
                                          playhead — see `seekTo`. Plain text
                                          again when there is nothing to play,
                                          so the app never offers an action it
                                          cannot perform. */}
                                      {audioPath ? (
                                        <button
                                          type="button"
                                          onClick={() => seekTo(s.start_ms, true)}
                                          aria-label={t("transcript.seek")}
                                          className={`rounded-xs tabular-nums hover:text-accent ${FOCUS}`}
                                        >
                                          {formatDuration(s.start_ms)}
                                        </button>
                                      ) : (
                                        <span className="tabular-nums">
                                          {formatDuration(s.start_ms)}
                                        </span>
                                      )}

                                    </div>
                                  )}
                                  {/* The corner nearest the speaker's own edge
                                      is cut to 6px — a tail without drawing a
                                      tail. `me` is a tint rather than a fill
                                      because `--color-me` is the accent, and a
                                      second accent fill would compete with the
                                      one on screen. */}
                                  <EditableLine
                                    text={s.text}
                                    mine={me}
                                    // Only the meeting being recorded is off
                                    // limits — the transcription ticker writes
                                    // its whole set back, so an edit there
                                    // would be overwritten by the next chunk.
                                    // Every other meeting is a finished
                                    // recording and correctable.
                                    editable={
                                      !status?.recording ||
                                      status.meeting_id !== selected.id
                                    }
                                    label={t("transcript.edit")}
                                    // Every segment, not only the one that
                                    // opens a turn: the turn's offset is the
                                    // only one drawn, and without this the
                                    // lines under it would be the part of the
                                    // transcript the recording cannot be
                                    // reached from.
                                    onSeek={
                                      audioPath
                                        ? () => seekTo(s.start_ms, false)
                                        : undefined
                                    }
                                    onSave={async (next) => {
                                      const updated =
                                        await api.editTranscriptSegment(
                                          selected.id,
                                          s.id,
                                          next,
                                        );
                                      setTranscript(updated);
                                    }}
                                  />
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
                        className="mx-auto max-w-pane"
                        data-testid="summary-panel"
                      >
                        {selected.summary ||
                        selected.key_points ||
                        selected.action_items ||
                        // A template whose sections are none of those three —
                        // a client call with Requirements and Risks and a model
                        // that skipped the summary — is still a summarised
                        // meeting, and without this it showed the empty state.
                        selected.sections?.length ? (
                          // One column of panels, full width, in the order the
                          // meeting is read: what happened, the points, the
                          // work, then the record of previous runs. It was a
                          // two-column grid, which spent a third of a wide
                          // screen on a rail while the prose stayed at 36rem
                          // and everything below the fold was in the narrow
                          // side. Down the page each panel gets the whole
                          // measure and nothing has to be hunted for.
                          <div className="space-y-4">
                            {/* The summary keeps its own shape inside the
                                panel: reading measure on the prose, and a
                                heavier weight than the panels under it, because
                                it is the answer and they are its parts. */}
                            <section className={`group ${PANEL}`}>
                              <div className="mb-2 flex items-start justify-between gap-2">
                                <h2 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
                                  {t("section.summary")}
                                </h2>
                                <CopyButton
                                  text={selected.summary || ""}
                                  label={t("section.summary")}
                                  className="-mt-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
                                />
                              </div>
                              <div className="max-w-reading">
                                <Markdown text={selected.summary || "—"} />
                              </div>
                            </section>
                            {/* Whatever else this template asked for, in its
                                order. `summary` is drawn above and `action_items`
                                below — one is the answer and the other is a list
                                of work with owners and a done flag, and neither
                                is a card of prose.

                                A meeting with no sections is one summarised
                                before templates had shapes of their own: it
                                falls back to Key points, which is what it has. */}
                            {selected.sections?.length ? (
                              selected.sections
                                .filter(
                                  (s) =>
                                    s.key !== "summary" &&
                                    s.key !== "action_items",
                                )
                                .map((s) => (
                                  <Section
                                    key={s.key}
                                    title={t(`section.${s.key}`)}
                                    body={s.body || "—"}
                                    onImprove={
                                      s.key === "key_points"
                                        ? () => improve("key_points")
                                        : undefined
                                    }
                                    improving={improving === "key_points"}
                                  />
                                ))
                            ) : (
                              <Section
                                title={t("section.key_points")}
                                body={selected.key_points || "—"}
                                onImprove={() => improve("key_points")}
                                improving={improving === "key_points"}
                              />
                            )}
                            {/* Not a `Section`: this one is work rather than
                                prose. It ticks off, carries an owner and a
                                deadline, and survives the meeting being
                                summarised again — which the free-text card
                                could not, because every run replaced it. The
                                panel chrome is out here so the three below the
                                summary read as one family. */}
                            <div className={PANEL}>
                              <ActionItems
                                key={selected.id}
                                meetingId={selected.id}
                                reloadKey={versionsKey}
                                // Only when there is something to play. Without
                                // a recording the stamp stays plain text rather
                                // than a button that does nothing.
                                onSeek={
                                  audioPath
                                    ? (ms) => seekTo(ms, true)
                                    : undefined
                                }
                              />
                            </div>
                            {/* Chrome inside this one, not out here: it returns
                                null once a meeting has a single version, and a
                                wrapper drawn around nothing is an empty bordered
                                box. Only the thing that knows it has content can
                                decide to draw a panel. */}
                            <SummaryHistory
                              meetingId={selected.id}
                              reloadKey={versionsKey}
                              onRestored={() => {
                                setVersionsKey((k) => k + 1);
                                void loadMeeting(selected.id);
                              }}
                            />
                          </div>
                        ) : summarizingId === selected.id ||
                          (progress?.phase === "summarizing" &&
                            progress.meeting_id === selected.id) ? (
                          <SummaryWorking />
                        ) : (
                          // Three cards each holding one em-dash and nothing to
                          // act on is an empty state. This renders it as one.
                          <Empty
                            icon={<Sparkles size={22} />}
                            title={t("empty.summary")}
                            body={t("empty.summary_body")}
                            action={
                              <div>
                                <Button
                                  size="md"
                                  onClick={() => handleSummarize("general")}
                                  disabled={busy}
                                >
                                  <Sparkles size={16} /> {t("action.summarize")}
                                </Button>
                                {/* Beside the button that fixes it, not in the
                                    global banner: the recording, the audio and
                                    the transcript are all saved and correct,
                                    and only this one step failed. A banner
                                    across the top would report a lost
                                    meeting. */}
                                {summaryFailed && (
                                  <p className="mt-3 text-xs text-danger">
                                    {t("processing.summary_failed")}
                                  </p>
                                )}
                              </div>
                            }
                          />
                        )}
                      </motion.div>
                    )}

                    {tab === "notes" && (
                      <motion.div
                        key="notes"
                        {...fadeRise}
                        className="mx-auto max-w-pane"
                        data-testid="notes-panel"
                      >
                        <NotesPanel key={selected.id} meetingId={selected.id} />
                      </motion.div>
                    )}

                    {tab === "chat" && (
                      // Keyed by the meeting so switching rows starts the
                      // conversation over rather than showing the previous
                      // meeting's history until the fetch lands.
                      <motion.div
                        key="chat"
                        {...fadeRise}
                        className="flex min-h-0 flex-1 flex-col"
                        data-testid="chat-panel"
                      >
                        <ChatPanel key={selected.id} meetingId={selected.id} />
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
                    <div>
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
                            {/* Named, always — the in-app listener above makes
                                the key true whether or not the OS granted the
                                global one. The literal is the fallback for a
                                backend that does not report the accelerator
                                yet, not a second claim about what is
                                registered. */}
                            <kbd className="inline-flex h-6 items-center rounded-xs border border-border bg-surface-2 px-2 font-sans text-2xs">
                              {shortcut?.accelerator ?? "Ctrl+Shift+R"}
                            </kbd>
                          </span>
                        )}
                      </div>
                      {/* Only when the backend says the OS refused it. The app
                          may not advertise a global shortcut it does not have,
                          and it may not stay silent about a key that works in
                          only half the cases the user will try. */}
                      {!status?.recording && shortcut?.registered === false && (
                        <p className="mt-2 text-2xs text-fg-subtle">
                          {t(shortcut.reason_key ?? "shortcut.unavailable")}
                        </p>
                      )}
                    </div>
                  }
                />
              </div>
            )}

            {/* The empty screen only. With a meeting open the transcript and the
                summary own this column, and a Record button hanging over them
                offers to start a second recording on top of the one the header
                is already showing.

                A running recording keeps it, whatever is selected: `handleStart`
                opens the new meeting straight away, so `!selected` stops being
                true the instant recording begins — which is precisely when the
                control has to still be there. It changes into pause and stop
                instead of vanishing. */}
            <AnimatePresence>
              {(!selected || status?.recording) && (
                <RecordDock
                  key="dock"
                  gate={gate}
                  busy={busy}
                  devices={devices}
                  micDeviceId={settings.mic_device_id}
                  systemDeviceId={settings.system_device_id}
                  micEnabled={settings.capture_me ?? true}
                  systemEnabled={settings.capture_others ?? true}
                  onStart={requestStart}
                  onOpenSettings={() => setShowSettings(true)}
                  onPickDevice={handlePickDevice}
                  recording={
                    status?.recording ? { paused: !!status.paused } : null
                  }
                  onStop={handleStop}
                  onPauseResume={handlePauseResume}
                >
                  {/* Above the transport and only while recording: it is context
                      about what is being said now, and after Stop the meeting's
                      own screen is where notes belong.

                      Inside the dock rather than beside it — the dock is
                      absolutely positioned and this was in the card's flow, so
                      the bar sat against the sidebar while the transport it
                      belongs to was centred. */}
                  {status?.recording && status.meeting_id && (
                    // The meeting being recorded, not the one on screen. The
                    // sidebar stays live during a recording, so those are not
                    // the same thing — and a note stamped with this recording's
                    // clock, filed against a meeting from last week, would be
                    // evidence of something that never happened.
                    <ContextBar meetingId={status.meeting_id} />
                  )}
                </RecordDock>
              )}
            </AnimatePresence>
          </div>
        </motion.main>
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

      {/* One at a time, oldest first. Two crashes are two decisions, and a
          stack of dialogs is a way to answer the wrong one. */}
      <AnimatePresence>
        {interrupted[0] && (
          <RecoveryDialog
            title={interrupted[0].title}
            onRecover={() => handleRecover(interrupted[0].id)}
            onDiscard={() => handleDiscardInterrupted(interrupted[0].id)}
            onLater={dismissRecovery}
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
            shortcut={shortcut}
            onShortcutChange={(next) => {
              setShortcut(next);
              // The snapshot too, not just the status. `set_record_shortcut`
              // writes the row itself, so leaving this stale meant a later Save
              // of any unrelated field sent the OLD combination back and undid
              // the change on the next launch.
              if (next.registered) {
                setSettings((s) => ({
                  ...s,
                  record_shortcut: next.accelerator,
                }));
              }
            }}
            onClose={() => setShowSettings(false)}
            onWiped={() => {
              // Everything the shell is holding is about to be about meetings
              // that no longer exist: the list, whatever is open, and the search
              // term that filtered it.
              setSelectedId(null);
              setQuery("");
              void refreshMeetings();
            }}
            onThemeChange={(theme) =>
              setSettings((prev) => ({ ...prev, theme }))
            }
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

function Section({
  title,
  body,
  onImprove,
  improving,
}: {
  title: string;
  body: string;
  /// Absent on a section that cannot be improved. The model is asked to extend
  /// one list at a time, so each section carries its own trigger rather than one
  /// button improving all three.
  onImprove?: () => void;
  improving?: boolean;
}) {
  const { t } = useI18n();
  return (
    // `group` so the copy button is quiet until the pointer is on the block it
    // copies. A control that is always lit competes with the text it sits on;
    // one that only exists on hover is unreachable — `focus-within` keeps it for
    // the keyboard.
    <section className={`group ${PANEL}`}>
      <div className="mb-2 flex items-start justify-between gap-2">
        <h2 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
          {title}
        </h2>
        <div className="-mr-1 -mt-1 flex shrink-0 items-center gap-1">
          {onImprove && (
            <Button
              size="xs"
              variant="ghost"
              onClick={onImprove}
              disabled={improving}
              className="opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
            >
              <Sparkles size={14} />
              {improving ? t("summary.improving") : t("summary.improve")}
            </Button>
          )}
          <CopyButton
            text={body}
            label={title}
            className="opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
          />
        </div>
      </div>
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
/// What the summary pane shows while the model is writing.
///
/// It used to show the empty state with its button greyed out: a click that
/// visibly did nothing for two seconds on a GPU, and closer to twenty on a
/// CPU. The three rows name the three cards that are coming, in the words they
/// will carry once they arrive, so the wait says what it is for.
///
/// No percentage. Generation reports no progress to report, and a bar that
/// invents one is worse than a wait that is honest about being a wait.
function SummaryWorking() {
  const { t } = useI18n();
  const sections = ["section.summary", "section.key_points", "section.action_items"];
  return (
    <div className="mx-auto max-w-md text-center">
      <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-lg bg-surface-2 text-fg-subtle">
        {/* Opacity alone, which is also the reduced-motion fallback — under
            `reducedMotion="user"` this keeps breathing instead of freezing
            into a still frame that reads as a hang. */}
        <motion.span
          className="flex"
          animate={{ opacity: [0.45, 1, 0.45] }}
          transition={{ duration: 1.6, repeat: Infinity, ease: "easeInOut" }}
        >
          <Sparkles size={22} />
        </motion.span>
      </div>
      <h2 className="text-lg font-semibold">{t("summary.working")}</h2>
      <p className="mt-2 text-base leading-relaxed text-fg-muted">{t("summary.working_body")}</p>
      <ul className="mt-6 space-y-2 text-left">
        {sections.map((key, i) => (
          <li key={key} className="flex items-center gap-3 rounded-lg bg-surface-2 px-3 py-2">
            <motion.span
              className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent"
              animate={{ opacity: [0.3, 1, 0.3] }}
              transition={{ duration: 1.6, repeat: Infinity, ease: "easeInOut", delay: i * 0.2 }}
            />
            <span className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
              {t(key)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

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
