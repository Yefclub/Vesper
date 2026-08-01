import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Mic, Settings } from "lucide-react";
import { AudioDevice, StartGate } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { distance, transition } from "../lib/motion";
import { Button, FOCUS } from "./Button";

interface Props {
  gate: StartGate;
  busy: boolean;
  devices: AudioDevice[];
  micDeviceId?: string | null;
  systemDeviceId?: string | null;
  onStart: () => void;
  onOpenSettings: () => void;
}

/**
 * The control that starts a recording, anchored to the bottom of the empty
 * screen — the only screen it appears on.
 *
 * It owns its own position: nothing above or beside it can push it. Once a
 * recording is running the transport moves to the header, where it outlives the
 * empty screen this dock lives on, and the dock leaves downward, towards the
 * edge it sits against.
 *
 * Depth comes from surface lightness and a border rather than a shadow. Shadows
 * read poorly on a near-black background, and the app should pick one technique
 * and keep it.
 */
export function RecordDock({
  gate,
  busy,
  devices,
  micDeviceId,
  systemDeviceId,
  onStart,
  onOpenSettings,
}: Props) {
  const { t } = useI18n();
  // Hover and focus are tracked apart: moving the mouse away while a control
  // inside is focused must not collapse the panel under the keyboard user.
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const expanded = hovered || focused;
  const blocked = !gate.allowed;
  // The translated key wins. The backend also ships an English sentence for
  // callers without a catalog, but in the app that sentence is the fallback,
  // not the answer — it was reaching the screen in English.
  const gateReason = gate.reason_key ? t(gate.reason_key) : (gate.reason ?? null);

  const deviceName = (id: string | null | undefined, kind: AudioDevice["kind"]) =>
    devices.find((d) => d.id === id)?.name ??
    devices.find((d) => d.kind === kind && d.is_default)?.name ??
    t("dock.system_default");

  return (
    // Leaves downward, towards the edge it is anchored to, so the eye is sent
    // where the dock lives rather than being asked to watch it dissolve in
    // place. Mounted inside an AnimatePresence, which is what runs the exit.
    <motion.div
      exit={{ opacity: 0, y: distance.md }}
      transition={transition.base}
      className="pointer-events-none absolute inset-x-0 bottom-0 flex flex-col items-center pb-6"
    >
      {/* The live region is mounted for the whole life of the dock, empty until
          there is something to say. A region inserted into the DOM together with
          its text is announced unreliably — screen readers only watch regions
          they were already aware of. Empty it has no box, so it costs no layout:
          the spacing below lives on the balloon, not on a gap here. */}
      <div role="status" className="flex flex-col items-center">
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
              initial={{ opacity: 0, y: 4, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: 4, scale: 0.98 }}
              transition={transition.base}
              className="pointer-events-auto relative mb-2 max-w-xs rounded-lg border border-border bg-surface-3 px-3 py-3"
            >
              <div className="flex gap-2">
                <span
                  aria-hidden
                  className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-danger/15 text-2xs font-semibold text-danger"
                >
                  !
                </span>
                <div className="flex flex-col items-start gap-2">
                  {/* The sentence is foreground, not red. Red is the accent that
                      says "look here"; a whole red paragraph just shouts. */}
                  <p className="text-xs leading-relaxed text-fg">{gateReason}</p>
                  <Button
                    variant="link"
                    className="text-xs text-accent"
                    onClick={onOpenSettings}
                  >
                    {t("gate.fix")}
                  </Button>
                </div>
              </div>
              {/* The arrow is the same surface and border as the balloon, rotated
                  45° and clipped so only the two outer edges show. Its tip
                  protrudes ~7px, which fits inside the balloon's 8px bottom
                  margin, so it points at the dock without overlapping it. */}
              <span
                aria-hidden
                className="absolute -bottom-[5px] left-1/2 h-2.5 w-2.5 -translate-x-1/2 rotate-45 border-b border-r border-border bg-surface-3"
              />
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* The gear sits beside the dock, not inside it, and the row is padded on
          the left by exactly what the gear and its gap take on the right. The
          padding is transparent background, so the dock stays a pill with no
          empty half — the previous balance was a spacer *inside* the dock, which
          bought the centring at the cost of a visible dead area next to Gravar.
          `items-start` keeps the gear level with the control row instead of
          drifting down when the device panel expands below. */}
      <div className="flex items-start gap-3 pl-[66px]">
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
          className="pointer-events-auto overflow-hidden rounded-lg border border-border bg-surface-2/95 backdrop-blur-md"
        >
          <div className="flex items-center justify-center gap-3 px-3 py-2">
            <Button
              size="md"
              data-testid="btn-record"
              disabled={busy || blocked}
              onClick={onStart}
            >
              <Mic size={16} aria-hidden /> {t("record.start")}
            </Button>
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
                <div className="flex items-center justify-center gap-4 px-4 py-2 text-2xs text-fg-muted">
                  <span className="flex items-center gap-2">
                    <span className="h-1.5 w-1.5 rounded-full bg-me" aria-hidden />
                    <span className="max-w-[14rem] truncate">
                      {deviceName(micDeviceId, "mic")}
                    </span>
                  </span>
                  <span className="flex items-center gap-2">
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

        {/* Its own pill, sized to the dock's collapsed height: 1px border plus
            p-2 either side of a 36px control is the same 54px the record row
            comes to. The 66px of padding on the row above is this pill plus the
            gap, so the dock — and Gravar inside it — keeps the screen's axis. */}
        <button
          data-testid="btn-dock-settings"
          onClick={onOpenSettings}
          title={t("nav.settings")}
          aria-label={t("nav.settings")}
          className={`group pointer-events-auto rounded-lg border border-border bg-surface-2/95 p-2 backdrop-blur-md ${FOCUS}`}
        >
          {/* group-hover, not hover: the padding is part of the target, and a
              hover that only lights up on the inner square feels dead at the
              pill's edges. */}
          <span className="flex h-9 w-9 items-center justify-center rounded-md bg-surface-3 text-fg-muted transition-colors group-hover:bg-hover group-hover:text-fg">
            <Settings size={16} />
          </span>
        </button>
      </div>
    </motion.div>
  );
}
