import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Pause, Play, Square } from "lucide-react";
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
  const [locale, setLocale] = useState("en");
  useEffect(() => {
    api
      .getSettings()
      .then((s) => setLocale(s.ui_locale || "en"))
      .catch(() => setLocale("en"));
  }, []);
  return (
    <I18nProvider initialLocale={locale}>
      <Card />
    </I18nProvider>
  );
}

function Card() {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
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
        setExpanded(true);
        void api.setOverlayExpanded(true);
      }}
      onMouseLeave={() => {
        setExpanded(false);
        void api.setOverlayExpanded(false);
      }}
      className="flex h-screen w-screen items-center justify-end"
    >
      <div className="flex h-full w-full flex-col overflow-hidden rounded-l-lg border border-r-0 border-border bg-surface-1 shadow-lift">
        {expanded ? (
          <>
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
          </>
        ) : (
          // Collapsed: a sliver with a pulsing dot. Enough to say a recording is
          // running and to give the pointer something to find.
          <div className="flex h-full w-full items-center justify-center">
            <span
              aria-hidden
              className={`h-2 w-2 rounded-full ${
                paused ? "bg-fg-subtle" : "animate-pulse bg-danger"
              }`}
            />
          </div>
        )}
      </div>
    </div>
  );
}
