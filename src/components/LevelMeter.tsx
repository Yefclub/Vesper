import { MicOff, MonitorX } from "lucide-react";
import type { ReactNode } from "react";
import { ChannelLevels, ChannelSelection } from "../lib/api";
import { useI18n } from "../lib/i18n";

/// `compact` is the header form: the two bars alone, short enough to sit in a
/// 56px header beside 32px controls. The labels go because the strip around it
/// already says what is being recorded.
///
/// `channels` is what keeps the meter honest. A bar at rest means one of two
/// things — nobody is talking, or nothing is listening — and only the recorder
/// knows which. Absent it reads as both channels on, which is what every
/// recording was before they could be switched off.
export function LevelMeter({
  levels,
  channels,
  compact = false,
}: {
  levels: ChannelLevels;
  channels?: ChannelSelection;
  compact?: boolean;
}) {
  const { t } = useI18n();
  const me = channels?.me ?? true;
  const others = channels?.others ?? true;
  const off = t("channel.off");
  const name = (label: string, on: boolean) => (on ? label : `${label} — ${off}`);
  const icon = compact ? 12 : 14;
  return (
    <div
      // The label was on a bare div, which drops it: a div has no role, so
      // there is nothing for the name to attach to.
      role="group"
      className="flex items-end gap-2"
      data-testid="level-meter"
      aria-label={`${name(t("level.me"), me)} / ${name(t("level.others"), others)}`}
    >
      <Bar
        label={t("level.me")}
        value={levels.me_rms}
        color="bg-me"
        compact={compact}
        // The same icon the dock's channel button carries when it is off, so
        // the state is recognised in the header without a second vocabulary.
        offIcon={me ? null : <MicOff size={icon} aria-hidden />}
      />
      <Bar
        label={t("level.others")}
        value={levels.others_rms}
        color="bg-others"
        compact={compact}
        offIcon={others ? null : <MonitorX size={icon} aria-hidden />}
      />
    </div>
  );
}

/**
 * RMS amplitude (0..1) to a fraction of the bar.
 *
 * Linear is why these bars never moved. Speech sits around 0.01–0.1 RMS, so
 * `value * 16` was 0.2–1.6px inside a `Math.max(4, …)` — the bar rendered at its
 * 4px floor whether the room was silent or someone was shouting into the
 * microphone. Loudness is logarithmic, and a meter has to be too.
 *
 * -60 dBFS is the floor and 0 dBFS the top, which puts ordinary speech in the
 * upper half of the bar where it can be seen to move.
 */
export function meterScale(rms: number): number {
  if (!(rms > 0)) return 0;
  const db = 20 * Math.log10(Math.min(1, rms));
  return Math.max(0, Math.min(1, (db + 60) / 60));
}

function Bar({
  label,
  value,
  color,
  compact,
  offIcon,
}: {
  label: string;
  value: number;
  color: string;
  compact: boolean;
  offIcon: ReactNode;
}) {
  const full = compact ? 16 : 28;
  const h = Math.max(2, Math.round(meterScale(value) * full));
  return (
    <div className="flex flex-col items-center gap-1">
      {offIcon ? (
        // The well is not drawn at all for a channel that is off. An empty one
        // is exactly what a silent room looks like, and the 2px floor of the
        // fill below would keep a sliver of colour lit under a channel nothing
        // is being captured on.
        <span
          className={`flex items-center justify-center text-fg-subtle ${
            compact ? "h-4" : "h-7"
          }`}
        >
          {offIcon}
        </span>
      ) : (
        <div
          className={`flex w-2 items-end overflow-hidden rounded-full bg-surface-3 ${
            compact ? "h-4" : "h-7"
          }`}
        >
          <div
            className={`w-full rounded-full ${color}`}
            // Height, not transform: the bar grows from the bottom of a 8-16px
            // well and there is nothing to composite against. The transition is
            // what turns ten samples a second into something that reads as a
            // level rather than as a flicker.
            style={{ height: h, transition: "height 90ms linear" }}
          />
        </div>
      )}
      {!compact && <span className="text-2xs text-fg-muted">{label}</span>}
    </div>
  );
}
