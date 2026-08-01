import { ChannelLevels } from "../lib/api";
import { useI18n } from "../lib/i18n";

/// `compact` is the header form: the two bars alone, short enough to sit in a
/// 56px header beside 32px controls. The labels go because the strip around it
/// already says what is being recorded.
export function LevelMeter({
  levels,
  compact = false,
}: {
  levels: ChannelLevels;
  compact?: boolean;
}) {
  const { t } = useI18n();
  return (
    <div
      // The label was on a bare div, which drops it: a div has no role, so
      // there is nothing for the name to attach to.
      role="group"
      className="flex items-end gap-2"
      data-testid="level-meter"
      aria-label={`${t("level.me")} / ${t("level.others")}`}
    >
      <Bar
        label={t("level.me")}
        value={levels.me_rms}
        color="bg-me"
        compact={compact}
      />
      <Bar
        label={t("level.others")}
        value={levels.others_rms}
        color="bg-others"
        compact={compact}
      />
    </div>
  );
}

function Bar({
  label,
  value,
  color,
  compact,
}: {
  label: string;
  value: number;
  color: string;
  compact: boolean;
}) {
  const full = compact ? 16 : 28;
  const h = Math.max(4, Math.min(full, Math.round(value * full)));
  return (
    <div className="flex flex-col items-center gap-1">
      <div
        className={`flex w-2 items-end overflow-hidden rounded-full bg-surface-3 ${
          compact ? "h-4" : "h-7"
        }`}
      >
        <div className={`w-full rounded-full ${color}`} style={{ height: h }} />
      </div>
      {!compact && <span className="text-2xs text-fg-muted">{label}</span>}
    </div>
  );
}
