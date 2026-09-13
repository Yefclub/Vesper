import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "framer-motion";
import { clsx } from "clsx";
import { LoaderCircle, Mic, Square } from "lucide-react";
import { api, DictationStatus } from "../lib/api";
import { transition } from "../lib/motion";
import { I18nProvider, useI18n } from "../lib/i18n";

/**
 * The dictation indicator: a sliver on the right edge of the screen that opens
 * into a microphone and a record button under the pointer.
 *
 * Its own window, and never focused — taking the keyboard from the application
 * being dictated into is the one thing it must not do. So it has no keyboard
 * route, and it is never the only way in: the shortcut, the tray and Vesper's
 * Dictations drawer all start, stop and recover a dictation.
 */
export function Indicator() {
  // Nothing renders until the locale is known, for the reason the meeting card
  // gives: `I18nProvider` reads its locale once, on mount.
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
      <Handle />
    </I18nProvider>
  );
}

/** How long controls opened by a finger stay open. A finger leaves no hover
 *  behind to close them. */
const TOUCH_OPEN_MS = 5000;

type Tone = "rest" | "live" | "busy" | "good" | "note" | "bad";

function toneOf(status: DictationStatus | null): Tone {
  switch (status?.state) {
    case "listening":
      return "live";
    case "transcribing":
    case "inserting":
      return "busy";
    case "inserted":
      return "good";
    // Kept without a failure is what was asked for; kept with one is not.
    case "saved":
      return status.reason_key ? "bad" : "good";
    case "unconfirmed":
      return "note";
    case "insertion_failed":
    case "transcription_failed":
      return "bad";
    default:
      // A refused start comes back idle, carrying its reason.
      return status?.reason_key ? "bad" : "rest";
  }
}

const FILL: Record<Tone, string> = {
  rest: "bg-success",
  live: "bg-danger",
  busy: "bg-warn",
  good: "bg-success",
  note: "bg-accent",
  bad: "bg-danger",
};

function Handle() {
  const { t } = useI18n();
  const [status, setStatus] = useState<DictationStatus | null>(null);
  const [expanded, setExpanded] = useState(false);
  /// Whether the pointer is still on the indicator. A ref: only the resize
  /// callback reads it, after the pointer may already have gone.
  const inside = useRef(false);
  const touchTimer = useRef<number | null>(null);
  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  useEffect(() => {
    let live = true;
    let heard = false;
    let un: (() => void) | null = null;
    listen<DictationStatus>("dictation://state", (e) => {
      heard = true;
      setStatus(e.payload);
    }).then((fn) => (live ? (un = fn) : fn()));
    // Only if no event beat it here: an answer that left before a change must
    // not paint over the change.
    api
      .dictationStatus()
      .then((s) => {
        if (live && !heard) setStatus(s);
      })
      .catch(() => {});
    // The window is hidden and shown, never destroyed. Hidden — during a
    // meeting, or with the setting off — the backend lets the controls go, and
    // this has to as well or it comes back drawing them in a sliver.
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        inside.current = false;
        setExpanded(false);
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      live = false;
      un?.();
      document.removeEventListener("visibilitychange", onVisibility);
      if (touchTimer.current !== null) window.clearTimeout(touchTimer.current);
    };
  }, []);

  function open() {
    // The window grows first and the controls animate only once it has: a
    // pill sliding into a 12px window is clipped to a sliver for the whole arc.
    void api
      .setDictationIndicatorExpanded(true)
      .catch(() => {})
      .finally(() => {
        if (inside.current || touchTimer.current !== null) {
          setExpanded(true);
          return;
        }
        // Gone before the window finished growing. Nothing will animate out of
        // it, so nothing would shrink it back.
        void api.setDictationIndicatorExpanded(false).catch(() => {});
      });
  }

  function holdOpenForTouch() {
    if (touchTimer.current !== null) window.clearTimeout(touchTimer.current);
    touchTimer.current = window.setTimeout(() => {
      touchTimer.current = null;
      if (!inside.current) setExpanded(false);
    }, TOUCH_OPEN_MS);
  }

  const tone = toneOf(status);
  const listening = status?.state === "listening";
  const busy = status?.state === "transcribing" || status?.state === "inserting";
  const said = status?.reason_key
    ? t(status.reason_key)
    : t(`dictation.state.${status?.state ?? "idle"}`);
  const action = t(listening ? "dictation.stop" : "dictation.start");

  return (
    <div
      onPointerEnter={(e) => {
        inside.current = true;
        if (e.pointerType === "touch") holdOpenForTouch();
        open();
      }}
      onPointerDown={(e) => {
        if (e.pointerType === "touch") holdOpenForTouch();
      }}
      onPointerLeave={(e) => {
        inside.current = false;
        // A lifted finger is a leave as well; the timer closes what it opened.
        if (e.pointerType !== "touch") setExpanded(false);
      }}
      className="relative flex h-screen w-screen items-center justify-end"
    >
      {/* The sliver, always mounted at its own footprint rather than the
          window's, so it does not step while the controls animate out of the
          larger window. 12 and 72 mirror `INDICATOR_COLLAPSED` in
          domain/dictation.rs; both footprints are centred on the same edge. */}
      <div
        aria-hidden
        // Hidden under the open controls, which say the same thing larger, and
        // back the moment they start to leave.
        className={clsx(
          "absolute right-0 top-1/2 flex h-[72px] w-[12px] -translate-y-1/2 items-center justify-center",
          expanded && "invisible",
        )}
      >
        <span
          className={clsx(
            "h-14 w-1.5 rounded-full border",
            tone === "rest"
              ? "border-fg-subtle bg-surface-1"
              : ["border-transparent", FILL[tone]],
            (tone === "live" || tone === "busy") && "animate-pulse",
          )}
        />
      </div>

      <AnimatePresence
        // The window shrinks only once the controls have finished leaving.
        onExitComplete={() =>
          void api.setDictationIndicatorExpanded(false).catch(() => {})
        }
      >
        {expanded && (
          <motion.div
            key="controls"
            // Out to the edge it lives on and back from it, on `x` and
            // `opacity` alone: this floats over the application being typed
            // into, and must not take frames from it.
            initial={reduce ? { opacity: 0 } : { opacity: 0, x: "100%" }}
            animate={{ opacity: 1, x: 0 }}
            exit={
              reduce
                ? { opacity: 0, transition: transition.exit }
                : { opacity: 0, x: "100%", transition: transition.panelExit }
            }
            transition={transition.panel}
            className="relative flex h-full w-full flex-col items-center justify-center gap-2"
          >
            <div
              role="status"
              title={said}
              className="flex h-[100px] w-12 items-center justify-center rounded-full bg-fg text-background shadow-lift"
            >
              {busy ? (
                <LoaderCircle
                  size={20}
                  aria-hidden
                  // A spin is motion; for someone who asked for none the
                  // same wait is said with opacity.
                  className={reduce ? "animate-pulse" : "animate-spin"}
                />
              ) : (
                <Mic
                  size={20}
                  aria-hidden
                  className={listening ? "animate-pulse" : undefined}
                />
              )}
              <span className="sr-only">{said}</span>
            </div>
            <button
              type="button"
              data-testid="dictation-indicator-toggle"
              disabled={busy}
              onClick={() => void api.toggleDictation("indicator").catch(() => {})}
              aria-label={action}
              title={action}
              className="relative flex h-11 w-11 items-center justify-center rounded-full border border-border bg-surface-3 text-fg shadow-lift transition-colors hover:bg-hover disabled:opacity-50"
            >
              {listening ? (
                <Square size={14} aria-hidden className="fill-current" />
              ) : (
                <span aria-hidden className="h-3 w-3 rounded-full bg-danger" />
              )}
              <span
                aria-hidden
                className={clsx(
                  "absolute right-0 top-0 h-2.5 w-2.5 rounded-full ring-2 ring-background",
                  FILL[tone],
                  (tone === "live" || tone === "busy") && "animate-pulse",
                )}
              />
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
