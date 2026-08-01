import { ChannelLevels } from "../lib/api";
import { useI18n } from "../lib/i18n";

export function LevelMeter({ levels }: { levels: ChannelLevels }) {
  const { t } = useI18n();
  return (
    <div
      className="flex items-end gap-2"
      data-testid="level-meter"
      aria-label={`${t("level.me")} / ${t("level.others")}`}
    >
      <Bar label={t("level.me")} value={levels.me_rms} color="bg-me" />
      <Bar label={t("level.others")} value={levels.others_rms} color="bg-others" />
    </div>
  );
}

function Bar({
  label,
  value,
  color,
}: {
  label: string;
  value: number;
  color: string;
}) {
  const h = Math.max(4, Math.min(28, Math.round(value * 28)));
  return (
    <div className="flex flex-col items-center gap-1">
      <div className="flex h-7 w-2 items-end overflow-hidden rounded-full bg-surface-3">
        <div className={`w-full rounded-full ${color}`} style={{ height: h }} />
      </div>
      <span className="text-2xs text-fg-muted">{label}</span>
    </div>
  );
}
