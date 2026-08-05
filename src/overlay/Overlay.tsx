import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Pause, Play, Square } from "lucide-react";
import { transition } from "../lib/motion";
import {
  api,
  formatDuration,
  LiveTranscript,
  RecorderStatus,
} from "../lib/api";
import { I18nProvider, useI18n } from "../lib/i18n";

/**
 * The card that docks to the right edge while the main window is minimized.
 *
 * It exists so a meeting can be watched and stopped without restoring the app
 * over whatever the user is actually in — a call, a shared screen. Collapsed it
 * is a sliver; the pointer expands it.
 *
 * Its own window, so it can float over other applications, and its own React
 * entry point rather than the main `App`: it needs the transcript, the clock and
 * two buttons, and mounting the whole shell to show them would run the sidebar,
 * the settings drawer and the meeting list in a 340px card nobody can see.
 */
export function Overlay() {
  // Nothing renders until the locale is known. `I18nProvider` reads
  // `initialLocale` once, on its first mount, so mounting it with a placeholder
  // and updating the state afterwards leaves the card in English for anyone
  // whose app is in Portuguese.
  const [locale, setLocale] = useState<string | null>(null);
  useEffect(() => {
    api
      .getSettings()
      .then((s) => setLocale(s.ui_locale || "en"))
      .catch(() => setLocale("en"));
  }, []);
  if (locale === null) return null;
  return (
    <I18nProvider initialLocale={locale}>
      <Card />
    </I18nProvider>
  );
}

function Card() {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  /// Whether the pointer is still on the card. A ref, not state: nothing
  /// renders from it — it only has to be readable by the resize callback that
  /// resolves after the pointer may already have gone.
  const inside = useRef(false);
  const reduced = useReducedMotion();
  const [status, setStatus] = useState<RecorderStatus | null>(null);
  const [transcript, setTranscript] = useState<LiveTranscript>({ segments: [] });
  const [busy, setBusy] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // The backend drives transcription now, so this window is a listener like the
  // main one. It cannot poll for lines: a minimized owner is exactly when the
  // page-hidden timer clamp bites, which is the bug that moved the loop out of
  // the window in the first place.
  useEffect(() => {
    const unsubs: Array<() => void> = [];
    let live = true;
    const track = (un: () => void) => (live ? unsubs.push(un) : un());
    (async () => {
      track(
        await listen<LiveTranscript>("transcript://append", (e) =>
          setTranscript(e.payload),
        ),
      );
    })();
    return () => {
      live = false;
      unsubs.forEach((u) => u());
    };
  }, []);

  // The clock and the pause state. Cheap — it reads atomics on the other side —
  // and it is the only thing that has to keep moving while nobody is speaking.
  useEffect(() => {
    const id = window.setInterval(() => {
      api
        .recorderStatus()
        .then(setStatus)
        .catch(() => {
          /* the window outliving the backend is not worth a banner */
        });
    }, 500);
    return () => window.clearInterval(id);
  }, []);

  // Newest line, always. Nobody scrolls a card they are hovering to read the
  // last thing said, so there is no pinned-to-bottom question here.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [transcript, expanded]);

  const last = transcript.segments?.slice(-12) ?? [];
  const paused = status?.paused ?? false;

  return (
    <div
      // The pointer is the whole interaction. `onFocus`/`onBlur` deliberately
      // absent: this window never takes focus — it must not steal it from the
      // meeting the user is in — so there is no keyboard path here to serve.
      onMouseEnter={() => {
        inside.current = true;
        // The window grows FIRST, and the card only animates once it has: a
        // card sliding into a 14px window is clipped to a sliver of itself for
        // the whole arc. `finally`, so a failed resize still opens rather than
        // leaving a card that never appears.
        void api
          .setOverlayExpanded(true)
          .catch(() => {})
          .finally(() => {
            // The pointer may already have left while that was in flight, and
            // opening behind it would leave the card up with nobody on it.
            if (inside.current) {
              setExpanded(true);
              return;
            }
            // Gone. The window is enlarged and nothing will ever animate out of
            // it, so `onExitComplete` will not fire — a pointer flicking across
            // the strip would leave a 340x268 invisible always-on-top rectangle
            // sitting over the user's screen until the next hover.
            void api.setOverlayExpanded(false);
          });
      }}
      onMouseLeave={() => {
        inside.current = false;
        setExpanded(false);
      }}
      className="relative flex h-screen w-screen items-center justify-end"
    >
      {/* The collapsed sliver, always mounted, at its own fixed footprint
          rather than at the window's. The window is 340×268 for as long as the
          card is animating out, and a sliver sized to the window would be seen
          at that size for those 200ms and then snap.

          `dock_right_center` centres both footprints on the monitor, so 132px
          centred inside 268px lands on the same pixels as a 132px window and
          the collapse is invisible — on any display tall enough to hold the
          expanded card, which is every display this ships to. Below 268px of
          height that function clamps to the top and only the collapsed window
          is still centred, so the sliver would step on the way down.

          132 and 14 mirror `COLLAPSED` in domain/overlay.rs, which says so. */}
      <div className="absolute right-0 top-1/2 flex h-[132px] w-[14px] -translate-y-1/2 items-center justify-center rounded-l-lg border border-r-0 border-border bg-surface-1 shadow-lift">
        <span
          aria-hidden
          className={`h-2 w-2 rounded-full ${
            paused ? "bg-fg-subtle" : "animate-pulse bg-danger"
          }`}
        />
      </div>

      <AnimatePresence
        // The window shrinks only once the card has finished leaving. Doing it
        // on `onMouseLeave` cut the exit off at its first frame.
        onExitComplete={() => void api.setOverlayExpanded(false)}
      >
        {expanded && (
          <motion.div
            key="card"
            // Out to the edge it lives on, and back from it. `x` and `opacity`
            // only, both composite-only: this window floats over whatever the
            // user is actually working in, and a layout-animating card here
            // would take frames from that.
            initial={reduced ? { opacity: 0 } : { opacity: 0, x: "100%" }}
            animate={{ opacity: 1, x: 0 }}
            exit={
              reduced
                ? { opacity: 0, transition: transition.exit }
                : { opacity: 0, x: "100%", transition: transition.panelExit }
            }
            transition={transition.panel}
            className="relative flex h-full w-full flex-col overflow-hidden rounded-l-lg border border-r-0 border-border bg-surface-1 shadow-lift"
          >
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
              <span
                aria-hidden
                className={`h-2 w-2 shrink-0 rounded-full ${
                  paused ? "bg-fg-subtle" : "animate-pulse bg-danger"
                }`}
              />
              <span className="text-2xs uppercase tracking-eyebrow text-fg-subtle">
                {paused ? t("record.paused_badge") : t("record.recording_badge")}
              </span>
              <span className="ml-auto text-xs tabular-nums text-fg-muted">
                {formatDuration(status?.elapsed_ms ?? 0)}
              </span>
            </div>

            <div
              ref={scrollRef}
              className="min-h-0 flex-1 space-y-2 overflow-y-auto px-3 py-2"
            >
              {last.length ? (
                last.map((s) => (
                  <p key={s.id} className="text-xs leading-relaxed">
                    <span
                      className={
                        s.speaker === "me" ? "text-me" : "text-others"
                      }
                    >
                      {s.speaker === "me"
                        ? t("speaker.me")
                        : t("speaker.others")}
                    </span>{" "}
                    <span className="text-fg-muted">{s.text}</span>
                  </p>
                ))
              ) : (
                <p className="text-xs text-fg-subtle">{t("overlay.listening")}</p>
              )}
            </div>

            <div className="flex items-center gap-2 border-t border-border px-3 py-2">
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  const call = paused
                    ? api.resumeRecording()
                    : api.pauseRecording();
                  call
                    .then(setStatus)
                    .catch(() => {})
                    .finally(() => setBusy(false));
                }}
                aria-label={paused ? t("record.resume") : t("record.pause")}
                className="rounded-md bg-surface-3 p-2 text-fg transition-colors hover:bg-hover disabled:opacity-50"
              >
                {paused ? <Play size={14} /> : <Pause size={14} />}
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  // Not awaited past the call: stopping runs the final
                  // transcription and the summary, and this window is closed by
                  // the backend the moment the recorder stops.
                  api
                    .stopRecording()
                    .catch(() => {})
                    .finally(() => setBusy(false));
                }}
                className="flex flex-1 items-center justify-center gap-1.5 rounded-md bg-danger/15 px-3 py-2 text-xs font-medium text-danger transition-colors hover:bg-danger/25 disabled:opacity-50"
              >
                <Square size={12} /> {t("record.stop")}
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
