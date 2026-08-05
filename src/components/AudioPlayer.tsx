import { useState, type RefObject } from "react";
import { clsx } from "clsx";
import { Pause, Play } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Button, FOCUS } from "./Button";
import { formatDuration } from "../lib/api";
import { useI18n } from "../lib/i18n";

/// The meeting's own recording, played where it was recorded.
///
/// The element is what everything on screen is read from: the state below is set
/// from its events rather than from the calls that cause them. That is what lets
/// a transcript line seek — which happens outside this component, through the
/// same ref — move the slider and flip the button without either side having to
/// know the other exists.
///
/// The path is not a path the window chose. It arrives from `meetingAudioPath`,
/// which is the only thing that adds a file to the asset protocol's scope, so a
/// URL naming anything else is refused before it is opened.
export function AudioPlayer({
  audioRef,
  path,
  durationMs,
  onUnavailable,
}: {
  audioRef: RefObject<HTMLAudioElement | null>;
  path: string;
  /// What the meeting row says it lasts. Used until the file says otherwise —
  /// see `total`.
  durationMs: number;
  /// The file stopped being readable. Deleting a recording while it is playing
  /// is the case this is for: the element errors, and the meeting goes back to
  /// showing that it has nothing to play rather than leaving a transport that
  /// does nothing.
  onUnavailable: () => void;
}) {
  const { t } = useI18n();
  const [playing, setPlaying] = useState(false);
  const [at, setAt] = useState(0);
  const [loaded, setLoaded] = useState(0);

  // The row's own figure until the file has said otherwise, and whenever it says
  // something unusable — a WAV whose header disagrees with its length reports a
  // duration of `NaN`, which collapses the slider to a point that cannot be
  // dragged.
  const total = loaded > 0 ? loaded : durationMs;

  const toggle = () => {
    const el = audioRef.current;
    if (!el) return;
    // Asked of the element rather than of `playing`: the button is not the only
    // thing that starts this, so its own idea of the state is a copy and the
    // element's is the original.
    if (el.paused) {
      void el.play().catch(() => {});
    } else {
      el.pause();
    }
  };

  return (
    <div className="flex items-center gap-3">
      <audio
        ref={audioRef}
        src={convertFileSrc(path)}
        // Metadata alone: opening a meeting must not pull a two-hour recording
        // off the disk to draw a slider. The range requests that follow fetch
        // only what is played.
        preload="metadata"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
        onError={onUnavailable}
        onLoadedMetadata={(e) => {
          const secs = e.currentTarget.duration;
          setLoaded(Number.isFinite(secs) ? secs * 1000 : 0);
        }}
        onTimeUpdate={(e) => setAt(e.currentTarget.currentTime * 1000)}
      />
      <Button
        size="icon"
        variant="ghost"
        onClick={toggle}
        aria-label={playing ? t("player.pause") : t("player.play")}
      >
        {playing ? <Pause /> : <Play />}
      </Button>
      <input
        type="range"
        min={0}
        max={Math.max(total, 1)}
        value={Math.min(at, total)}
        aria-label={t("player.position")}
        aria-valuetext={formatDuration(at)}
        onChange={(e) => {
          const ms = Number(e.currentTarget.value);
          // Moved here as well as on the element: `timeupdate` does not fire
          // while the audio is paused, so without this the thumb springs back
          // under the pointer on every drag of a paused recording.
          setAt(ms);
          const el = audioRef.current;
          if (el) el.currentTime = ms / 1000;
        }}
        className={clsx("h-1 min-w-0 flex-1 cursor-pointer accent-accent", FOCUS)}
      />
      <span className="shrink-0 tabular-nums text-xs text-fg-muted">
        {formatDuration(at)} / {formatDuration(total)}
      </span>
    </div>
  );
}
