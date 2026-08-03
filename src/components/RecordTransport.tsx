import { Pause, Play, Square } from "lucide-react";
import { RecorderStatus, formatDuration } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { Button, FOCUS_DANGER } from "./Button";
import { LevelMeter } from "./LevelMeter";

interface Props {
  status: RecorderStatus;
  busy: boolean;
  onStop: () => void;
  onPauseResume: () => void;
}

/**
 * The controls for a recording in progress, in the header.
 *
 * They used to live in the dock at the bottom of the content column — which is
 * on screen only while no meeting is open, and starting a recording opens one.
 * Up here they survive every screen the user can reach mid-capture, so Stop is
 * never behind a navigation.
 *
 * The whole strip is one live region. The change from recording to paused is
 * the thing that has to be announced, and it is the only text in here that
 * moves for a reason other than the clock ticking.
 */
export function RecordTransport({ status, busy, onStop, onPauseResume }: Props) {
  const { t } = useI18n();
  const paused = status.paused;
  const pauseLabel = paused ? t("record.resume") : t("record.pause");

  return (
    // The badge, the clock and the meter are readouts, not controls, and this
    // sits in the titlebar: anything here that answers a press is a piece of
    // window the user cannot drag from. They are transparent, the buttons
    // below opt back in.
    <div
      data-testid="recording-badge"
      role="status"
      className="pointer-events-none flex items-center gap-3 [&_button]:pointer-events-auto"
    >
      {/* Live and paused are two states, not one word swapped: a filled dot
          pulsing in danger against a hollow, static one in muted. Before this
          the pill, the dot and the colour were identical in both. */}
      <span
        className={`flex h-6 items-center gap-2 rounded-full px-3 text-xs ${
          paused ? "bg-surface-2 text-fg-muted" : "bg-danger/10 text-danger"
        }`}
      >
        <span
          aria-hidden
          className={`h-1.5 w-1.5 rounded-full ${
            paused
              ? "border border-current"
              : "animate-pulse bg-danger motion-reduce:animate-none"
          }`}
        />
        {paused ? t("record.paused_badge") : t("record.recording_badge")}
      </span>

      {/* aria-hidden on purpose: inside the live region the digits would be
          read out every second. The state is on the badge beside them, and
          tabular-nums keeps a changing digit from shifting the controls. */}
      <span
        data-testid="elapsed"
        aria-hidden
        className="min-w-[7ch] text-center text-sm tabular-nums text-fg-muted"
      >
        {formatDuration(status.elapsed_ms)}
      </span>

      <LevelMeter levels={status.levels} compact />

      <Button
        variant="ghost"
        size="icon"
        data-testid="btn-pause"
        onClick={onPauseResume}
        title={pauseLabel}
        aria-label={pauseLabel}
      >
        {paused ? <Play size={16} /> : <Pause size={16} />}
      </Button>

      {/* Soft red, hand-rolled rather than variant="danger": there is one solid
          red in the app and it belongs on the delete dialog. `ghost` cannot
          carry it either — ghost sets text-fg-muted, and two text colours on
          one element are settled by the stylesheet's emission order rather
          than by this file. */}
      <button
        type="button"
        data-testid="btn-stop"
        onClick={onStop}
        disabled={busy}
        className={`inline-flex h-8 items-center gap-2 rounded-md bg-danger/15 px-3 text-sm font-medium text-danger transition-colors hover:bg-danger/25 disabled:pointer-events-none disabled:opacity-50 ${FOCUS_DANGER}`}
      >
        <Square size={16} aria-hidden /> {t("record.stop")}
      </button>
    </div>
  );
}
