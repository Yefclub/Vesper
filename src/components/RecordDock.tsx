import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Mic, Pause, Play, Settings, Square } from "lucide-react";
import { AudioDevice, RecorderStatus, StartGate, formatDuration } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { transition } from "../lib/motion";
import { LevelMeter } from "./LevelMeter";

interface Props {
  status: RecorderStatus | null;
  gate: StartGate;
  busy: boolean;
  devices: AudioDevice[];
  micDeviceId?: string | null;
  systemDeviceId?: string | null;
  onStart: () => void;
  onStop: () => void;
  onPauseResume: () => void;
  onOpenSettings: () => void;
}

/**
 * The recording control, anchored to the bottom of the content column.
 *
 * It used to live in the header, centred with `mx-auto` inside a flex row — which
 * centres between siblings, not in the container, so it sat off to one side and
 * moved again whenever the recording state changed the width of either
 * neighbour. Down here it owns its own position: nothing above or beside it can
 * push it, and the idle, hover and recording states all render into the same
 * anchored box.
 *
 * Depth comes from surface lightness and a border rather than a shadow. Shadows
 * read poorly on a near-black background, and the app should pick one technique
 * and keep it.
 */
export function RecordDock({
  status,
  gate,
  busy,
  devices,
  micDeviceId,
  systemDeviceId,
  onStart,
  onStop,
  onPauseResume,
  onOpenSettings,
}: Props) {
  const { t } = useI18n();
  // Hover and focus are tracked apart: moving the mouse away while a control
  // inside is focused must not collapse the panel under the keyboard user.
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const expanded = hovered || focused;
  const recording = status?.recording ?? false;
  const blocked = !gate.allowed && !recording;
  // The translated key wins. The backend also ships an English sentence for
  // callers without a catalog, but in the app that sentence is the fallback,
  // not the answer — it was reaching the screen in English.
  const gateReason = gate.reason_key ? t(gate.reason_key) : (gate.reason ?? null);

  const deviceName = (id: string | null | undefined, kind: AudioDevice["kind"]) =>
    devices.find((d) => d.id === id)?.name ??
    devices.find((d) => d.kind === kind && d.is_default)?.name ??
    t("dock.system_default");

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 flex flex-col items-center gap-2 pb-6">
      {/* Anchored above the control it explains, with an arrow pointing at it —
          feedback belongs next to its trigger, not stacked underneath in a block
          that stretches the dock and drags the button off centre.
          Rendered whenever recording is blocked rather than on hover: the button
          it describes is disabled, and a disabled control takes neither focus nor
          a tooltip, so hover would hide this from keyboard and touch entirely. */}
      <AnimatePresence>
        {blocked && gateReason && (
          <motion.div
            data-testid="record-gate"
            role="status"
            initial={{ opacity: 0, y: 4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.98 }}
            transition={transition.base}
            className="pointer-events-auto relative max-w-xs rounded-xl border border-border bg-surface-3 px-3.5 py-3 shadow-none"
          >
            <div className="flex gap-2.5">
              <span
                aria-hidden
                className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-danger/15 text-[10px] font-semibold text-danger"
              >
                !
              </span>
              <div className="flex flex-col items-start gap-2">
                {/* The sentence is foreground, not red. Red is the accent that
                    says "look here"; a whole red paragraph just shouts. */}
                <p className="text-xs leading-relaxed text-foreground">{gateReason}</p>
                <button
                  onClick={onOpenSettings}
                  className="text-xs font-medium text-accent underline-offset-2 transition-colors hover:underline focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-3),0_0_0_4px_var(--color-accent)]"
                >
                  {t("gate.fix")}
                </button>
              </div>
            </div>
            {/* The arrow is the same surface and border as the balloon, rotated
                45° and clipped so only the two outer edges show. */}
            <span
              aria-hidden
              className="absolute -bottom-[5px] left-1/2 h-2.5 w-2.5 -translate-x-1/2 rotate-45 border-b border-r border-border bg-surface-3"
            />
          </motion.div>
        )}
      </AnimatePresence>

      <motion.div
        layout
        transition={transition.base}
        onHoverStart={() => setHovered(true)}
        onHoverEnd={() => setHovered(false)}
        onFocusCapture={() => setFocused(true)}
        onBlurCapture={(e) => {
          // Only clear once focus has left the dock entirely, or tabbing between
          // its own buttons would close it under the user.
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setFocused(false);
        }}
        data-testid="record-dock"
        className="pointer-events-auto overflow-hidden rounded-2xl border border-border bg-surface-2/95 backdrop-blur-md"
      >
        <div className="flex items-center justify-center gap-3 px-3 py-2.5">
          {!recording ? (
            <button
              data-testid="btn-record"
              disabled={busy || blocked}
              onClick={onStart}
              className="flex items-center gap-2 rounded-xl bg-accent px-4 py-2 text-sm font-medium text-black transition-[filter,opacity] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-accent)]"
            >
              <Mic size={16} aria-hidden /> {t("record.start")}
            </button>
          ) : null}
          {!recording && (
            /* Beside the primary action, not stacked under a paragraph. Same
               height and radius so the pair reads as one control group. */
            <button
              data-testid="btn-dock-settings"
              onClick={onOpenSettings}
              title={t("nav.settings")}
              aria-label={t("nav.settings")}
              className="rounded-xl bg-surface-3 p-2 text-muted transition-colors hover:bg-border hover:text-foreground focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-accent)]"
            >
              <Settings size={16} />
            </button>
          )}
          {recording && (
            <>
              <button
                data-testid="btn-stop"
                onClick={onStop}
                className="flex items-center gap-2 rounded-xl bg-danger/15 px-4 py-2 text-sm font-medium text-danger transition-colors hover:bg-danger/25 focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-danger)]"
              >
                <Square size={14} aria-hidden /> {t("record.stop")}
              </button>
              <button
                data-testid="btn-pause"
                onClick={onPauseResume}
                aria-label={status?.paused ? t("record.resume") : t("record.pause")}
                className="rounded-xl bg-surface-3 p-2 text-foreground transition-colors hover:bg-border focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-accent)]"
              >
                {status?.paused ? <Play size={16} /> : <Pause size={16} />}
              </button>
              {/* tabular-nums so the elapsed time does not shift the controls
                  beside it every time a digit changes width. */}
              <span
                data-testid="elapsed"
                className="min-w-[5ch] text-center text-sm tabular-nums text-muted"
              >
                {formatDuration(status?.elapsed_ms ?? 0)}
              </span>
              {status && <LevelMeter levels={status.levels} />}
            </>
          )}
        </div>

        {/* The lower panel is the device readout and nothing else now. The
            blocked reason moved out to a balloon, so a long sentence no longer
            stretches the dock and drags the button off centre. */}
        <AnimatePresence initial={false}>
          {expanded && !blocked && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={transition.base}
              className="border-t border-border/60"
            >
              <div className="flex items-center justify-center gap-4 px-4 py-2.5 text-[11px] text-muted">
                <span className="flex items-center gap-1.5">
                  <span className="h-1.5 w-1.5 rounded-full bg-me" aria-hidden />
                  <span className="max-w-[14rem] truncate">
                    {deviceName(micDeviceId, "mic")}
                  </span>
                </span>
                <span className="flex items-center gap-1.5">
                  <span className="h-1.5 w-1.5 rounded-full bg-others" aria-hidden />
                  <span className="max-w-[14rem] truncate">
                    {deviceName(systemDeviceId, "system")}
                  </span>
                </span>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </div>
  );
}
