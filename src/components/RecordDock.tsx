import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Mic, Pause, Play, Square } from "lucide-react";
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
}: Props) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const recording = status?.recording ?? false;
  const blocked = !gate.allowed && !recording;

  const deviceName = (id: string | null | undefined, kind: AudioDevice["kind"]) =>
    devices.find((d) => d.id === id)?.name ??
    devices.find((d) => d.kind === kind && d.is_default)?.name ??
    t("dock.system_default");

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center pb-6">
      <motion.div
        layout
        transition={transition.base}
        onHoverStart={() => setExpanded(true)}
        onHoverEnd={() => setExpanded(false)}
        onFocusCapture={() => setExpanded(true)}
        onBlurCapture={(e) => {
          // Only collapse once focus has left the dock entirely, or tabbing
          // between its own buttons would close it under the user.
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setExpanded(false);
        }}
        data-testid="record-dock"
        className="pointer-events-auto overflow-hidden rounded-2xl border border-border bg-surface-2/95 backdrop-blur-md"
      >
        <div className="flex items-center gap-3 px-3 py-2.5">
          {!recording ? (
            <button
              data-testid="btn-record"
              disabled={busy || blocked}
              onClick={onStart}
              className="flex items-center gap-2 rounded-xl bg-accent px-4 py-2 text-sm font-medium text-black transition-[filter,opacity] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-accent)]"
            >
              <Mic size={16} aria-hidden /> {t("record.start")}
            </button>
          ) : (
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

        <AnimatePresence initial={false}>
          {expanded && (
            <motion.div
              // Height animates through the grid trick rather than `height:auto`,
              // which is not animatable; only transform and opacity actually move.
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={transition.base}
              className="border-t border-border/60"
            >
              <div className="flex items-center gap-4 px-4 py-2.5 text-[11px] text-muted">
                {blocked && gate.reason ? (
                  <span data-testid="record-gate" className="max-w-xs text-danger">
                    {gate.reason}
                  </span>
                ) : (
                  <>
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
                  </>
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </div>
  );
}
