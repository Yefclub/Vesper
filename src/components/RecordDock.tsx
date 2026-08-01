import { useState } from "react";
import { AnimatePresence, motion, type Variants } from "framer-motion";
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
 * The gear slot's width in px, mirroring `--dock-gear-offset` in
 * styles/index.css: 1px border ×2 + 4px pad ×2 + 36px control + 12px gap.
 *
 * A number rather than the token itself because framer interpolates numbers —
 * animating `0` to `var(--dock-gear-offset)` mixes a unitless zero with a rem
 * string and snaps instead of easing. Change one, change the other.
 */
const SLOT = 58;

/**
 * One interpolation, both directions.
 *
 * The expansion used to be an `AnimatePresence` mount and a `layout` animation,
 * so it grew on the way in and cut on the way out. Driving both ends off the
 * same variant means framer retargets from wherever the value had got to:
 * pulling the mouse away halfway through closes from halfway, not from the end.
 *
 * Width, height, opacity and scale only. `marginLeft`, `maxHeight`, `padding`,
 * `clipPath` and `filter` would each produce the same reveal, and each of them
 * is exempt from the snap `MotionConfig reducedMotion="user"` applies in
 * App.tsx — half the reveal would honour the setting and half would not. For
 * the same reason there is no `useReducedMotion()` here: these properties are
 * already covered, and opacity is still allowed to fade.
 */
const slot: Variants = {
  collapsed: { width: 0 },
  expanded: { width: SLOT },
};

const gearIn: Variants = {
  collapsed: { opacity: 0, scale: 0.92 },
  expanded: { opacity: 1, scale: 1 },
};

/**
 * Width as well as height.
 *
 * `height: 0` hides the readout but leaves it setting the shell's *intrinsic
 * width*: a pill holding one 110px button rendered 455px wide, because two
 * device names were measuring it from inside a zero-height box. Collapsing the
 * width too puts the shell back on the control row, which is the whole point of
 * a dock that is only Record at rest. Still width/height/opacity only, so
 * `MotionConfig reducedMotion="user"` snaps all of it.
 */
const panel: Variants = {
  collapsed: { height: 0, width: 0, opacity: 0 },
  expanded: { height: "auto", width: "auto", opacity: 1 },
};

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

      {/* The gear is part of the dock's expansion now, not a permanent pill
          beside it. Nothing but Record is on screen at rest; arriving with the
          pointer or with Tab grows a slot on the right for the gear and an
          empty one of the same width on the left, so Record keeps the screen's
          axis instead of sliding while the row grows.

          The hover surface is this row rather than the dock pill: the pointer
          has to cross the gap to reach the gear, and a surface that stopped at
          the pill's edge would close the thing it was travelling towards. */}
      <motion.div
        initial="collapsed"
        animate={expanded ? "expanded" : "collapsed"}
        transition={transition.base}
        onHoverStart={() => setHovered(true)}
        onHoverEnd={() => setHovered(false)}
        onFocusCapture={() => setFocused(true)}
        onBlurCapture={(e) => {
          // Only clear once focus has left the dock entirely, or tabbing between
          // its own buttons would close it under the user.
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setFocused(false);
        }}
        className="flex"
      >
        {/* The counterweight: empty, unreachable, and the only reason Record's
            centre stays put while the gear grows opposite it. */}
        <motion.div aria-hidden variants={slot} className="shrink-0" />

        <motion.div
          initial="collapsed"
          // Off the same hover, but not the same condition: the gear has to
          // appear while recording is blocked — it is the way out of the block —
          // while the device readout must not, because the balloon explaining
          // the block is already open above it.
          animate={expanded && !blocked ? "expanded" : "collapsed"}
          transition={transition.base}
          data-testid="record-dock"
          className="pointer-events-auto overflow-hidden rounded-lg border border-border border-t-border-strong bg-surface-2/95 p-1 backdrop-blur-md"
        >
          {/* 4px of shell around a 36px control: 12px outer radius minus the
              4px inset is exactly the 8px the button inside carries. */}
          <div className="flex items-center justify-center">
            <Button
              size="md"
              data-testid="btn-record"
              disabled={busy || blocked}
              onClick={onStart}
            >
              <Mic size={16} aria-hidden /> {t("record.start")}
            </Button>
          </div>

          {/* The lower panel is the device readout and nothing else. The blocked
              reason moved out to a balloon, so a long sentence no longer
              stretches the dock and drags the button off centre.
              Never unmounted, so closing is the opening played backwards; it
              clips itself rather than relying on the shell, whose 4px of bottom
              padding would otherwise leak a sliver of text while collapsed. */}
          <motion.div
            variants={panel}
            className="overflow-hidden border-t border-border/60"
          >
            {/* w-max so the row keeps its natural width while the box around it
                animates from zero, instead of reflowing the names on every frame. */}
            <div className="flex w-max items-center justify-center gap-4 px-4 py-2 text-2xs text-fg-muted">
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
        </motion.div>

        {/* The slot carries the width; the gear floats inside it, absolutely
            placed at the gap so a growing box never squeezes or clips it. The
            slot also carries the pointer events, because an element at opacity 0
            still takes clicks — and taking them off the button itself would have
            left the gear invisible but pressable across the whole 58px. */}
        <motion.div
          variants={slot}
          style={{ pointerEvents: expanded ? "auto" : "none" }}
          className="relative shrink-0"
        >
          {/* Same shell as the dock — 1px border, p-1, a 36px control — so the
              two pills are the same 46px by construction rather than by
              comment. group-hover, not hover: the padding is part of the
              target, and a hover that only lights up on the inner square feels
              dead at the pill's edges. */}
          <motion.button
            variants={gearIn}
            data-testid="btn-dock-settings"
            onClick={onOpenSettings}
            title={t("nav.settings")}
            aria-label={t("nav.settings")}
            className={`group absolute left-3 top-0 rounded-lg border border-border border-t-border-strong bg-surface-2/95 p-1 backdrop-blur-md ${FOCUS}`}
          >
            <span className="flex h-9 w-9 items-center justify-center rounded-md bg-surface-3 text-fg-muted transition-colors group-hover:bg-hover group-hover:text-fg">
              <Settings size={16} />
            </span>
          </motion.button>
        </motion.div>
      </motion.div>
    </motion.div>
  );
}
