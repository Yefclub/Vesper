import { ChannelLevels } from "../lib/api";

export function LevelMeter({ levels }: { levels: ChannelLevels }) {
  return (
    <div
      className="flex items-end gap-2"
      data-testid="level-meter"
      title="Me / Others levels"
    >
      <Bar label="Me" value={levels.me_rms} color="bg-me" />
      <Bar label="Others" value={levels.others_rms} color="bg-others" />
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
      <span className="text-[9px] text-muted">{label}</span>
    </div>
  );
}
