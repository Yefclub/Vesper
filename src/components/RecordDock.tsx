import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { AnimatePresence, motion, type Variants } from "framer-motion";
import { Check, Mic, MonitorSpeaker, Pause, Play, Settings, Square } from "lucide-react";
import { clsx } from "clsx";
import { AudioDevice, StartGate } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { distance, transition } from "../lib/motion";
import { Button } from "./Button";

type DeviceKind = AudioDevice["kind"];

/** A row in a channel's popover. `null` is the system default, the same value
 *  the drawer's empty `<option>` writes. */
interface DeviceOption {
  id: string | null;
  name: string;
}

interface Props {
  gate: StartGate;
  busy: boolean;
  devices: AudioDevice[];
  micDeviceId?: string | null;
  systemDeviceId?: string | null;
  onStart: () => void;
  /// Present while a recording is running. The dock keeps its place and its
  /// centre control changes hands — the button the user pressed to start is
  /// where they will look to stop.
  recording?: { paused: boolean } | null;
  onStop?: () => void;
  onPauseResume?: () => void;
  onOpenSettings: () => void;
  onPickDevice: (kind: DeviceKind, id: string | null) => void;
}

/**
 * The width of each side cell in px, mirroring `--dock-side` in
 * styles/index.css: 8px lead gap + 32px control + 4px gap + 32px control.
 *
 * A number rather than the token itself because framer interpolates numbers —
 * animating `0` to `var(--dock-side)` mixes a unitless zero with a rem string
 * and snaps instead of easing. Change one, change the other.
 */
const DOCK_SIDE = 76;

/**
 * Both side cells, off one variant — which is what nails Record's centre to the
 * screen's axis at every frame of the animation rather than at the two ends of
 * it. The left cell carries the device buttons, the right cell carries the
 * gear, and they are the same width by construction.
 *
 * One interpolation, both directions: driving open and closed off the same
 * variant means framer retargets from wherever the value had got to, so pulling
 * the mouse away halfway through closes from halfway rather than from the end.
 *
 * Width and opacity only. `marginLeft`, `maxWidth`, `padding`, `clipPath` and
 * `filter` would each produce the same reveal, and each of them is exempt from
 * the snap `MotionConfig reducedMotion="user"` applies in App.tsx — half the
 * reveal would honour the setting and half would not. For the same reason
 * there is no `useReducedMotion()` here.
 */
const side: Variants = {
  collapsed: { width: 0, opacity: 0 },
  expanded: { width: DOCK_SIDE, opacity: 1 },
};

/**
 * The control that starts a recording, anchored to the bottom of the empty
 * screen — the only screen it appears on.
 *
 * It owns its own position: nothing above or beside it can push it. The dock's
 * height is constant by construction — 4px of shell, a 36px control, 4px of
 * shell — and nothing is ever below the Record row, so expanding cannot move
 * Record in y. Only the width changes, and it changes symmetrically, so it
 * cannot move Record in x either.
 *
 * Once a recording is running the transport moves to the header, where it
 * outlives the empty screen this dock lives on, and the dock leaves downward,
 * towards the edge it sits against.
 *
 * Depth comes from surface lightness and a border rather than a shadow. Nothing
 * scrolls under the dock — it renders only on the empty screen — so it needs
 * neither a blur nor an occlusion shadow to separate itself from anything.
 */
export function RecordDock({
  gate,
  busy,
  devices,
  micDeviceId,
  systemDeviceId,
  onStart,
  onOpenSettings,
  onPickDevice,
  recording,
  onStop,
  onPauseResume,
}: Props) {
  const { t } = useI18n();
  // Hover and focus are tracked apart: moving the mouse away while a control
  // inside is focused must not collapse the dock under the keyboard user.
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  // Which channel's list is open, if any. One piece of state for both, so
  // opening one closes the other and the dock has a single term to read.
  const [menu, setMenu] = useState<DeviceKind | null>(null);
  // A popover opened with the mouse keeps focus on its trigger, so `focused`
  // covers it in practice. `menu` is the explicit guarantee — a dock that
  // collapsed under an open list would take the list with it.
  const expanded = hovered || focused || menu !== null;
  const blocked = !gate.allowed;
  // The translated key wins. The backend also ships an English sentence for
  // callers without a catalog, but in the app that sentence is the fallback,
  // not the answer — it was reaching the screen in English.
  const gateReason = gate.reason_key ? t(gate.reason_key) : (gate.reason ?? null);

  const deviceName = (id: string | null | undefined, kind: AudioDevice["kind"]) =>
    devices.find((d) => d.id === id)?.name ??
    devices.find((d) => d.kind === kind && d.is_default)?.name ??
    t("dock.system_default");

  const optionsFor = (kind: DeviceKind): DeviceOption[] => [
    // The explicit first row, matching the drawer's empty <option>: without it
    // a null setting reads as the first device in the list, so the screen would
    // claim a choice the backend has not been given.
    { id: null, name: t("dock.system_default") },
    ...devices
      .filter((d) => d.kind === kind)
      .map((d) => ({ id: d.id, name: d.name })),
  ];

  // Stable, so each popover binds its document-level pointerdown listener once
  // per opening rather than once per render of the dock.
  const setMenuOpen = useCallback(
    (kind: DeviceKind, open: boolean) => setMenu(open ? kind : null),
    [],
  );

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
          {blocked && gateReason && !recording && (
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

      {/* One pill, one border, one fill: the gear used to be a second pill 12px
          to the right of this one, which is the "separado da caixa" complaint,
          and the device readout used to be a panel below Record, which is what
          pushed Record up when it opened.

          The hover surface is the pill itself now rather than a wider row,
          because there is no longer a gap between two boxes for the pointer to
          cross.

          Both side cells expand while blocked, not just the gear: they animate
          off one variant, and gating one of them on `blocked` would move Record
          by 76px exactly when the balloon is asking the user to go and fix
          something. The device buttons are part of that fix. */}
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
        data-testid="record-dock"
        className="pointer-events-auto flex items-center rounded-lg border border-border bg-surface-2 p-1"
      >
        {/* Clipped so the trailing pad disappears at width 0; the buttons are
            `shrink-0`, so they are clipped rather than squashed on the way. The
            clip has to lift while a list is open or it would cut the popover
            off at the cell's edge — safe, because a list can only be opened
            from an already-expanded dock, where nothing overflows. */}
        {/* Gone while recording. `start_recording` copies both device ids into
            the streams, so picking another one here would write a setting and
            change nothing about the capture that is running — a control that
            answers and does nothing is worse than no control. */}
        <motion.div
          variants={side}
          className={clsx(
            "relative flex shrink-0 items-center gap-1 pr-2",
            recording && "hidden",
            menu === null ? "overflow-hidden" : "overflow-visible",
          )}
        >
          <DeviceButton
            kind="mic"
            name={deviceName(micDeviceId, "mic")}
            options={optionsFor("mic")}
            current={micDeviceId ?? null}
            open={menu === "mic"}
            onOpenChange={setMenuOpen}
            onPick={onPickDevice}
          />
          <DeviceButton
            kind="system"
            name={deviceName(systemDeviceId, "system")}
            options={optionsFor("system")}
            current={systemDeviceId ?? null}
            open={menu === "system"}
            onOpenChange={setMenuOpen}
            onPick={onPickDevice}
          />
        </motion.div>

        {/* 4px of shell around a 36px control: 12px outer radius minus the 4px
            inset is exactly the 8px the button inside carries.

            While a recording runs the same slot holds pause and stop. The
            header shows the transport too, and both are wanted: the header is
            where the elapsed time and the meters live, and this is where the
            hand already is — it is the button that started the recording. */}
        {recording ? (
          <div className="flex items-center gap-2">
            <Button
              variant="secondary"
              size="md"
              data-testid="btn-dock-pause"
              disabled={busy}
              onClick={onPauseResume}
            >
              {recording.paused ? (
                <Play size={16} aria-hidden />
              ) : (
                <Pause size={16} aria-hidden />
              )}
              {t(recording.paused ? "record.resume" : "record.pause")}
            </Button>
            <Button
              size="md"
              data-testid="btn-dock-stop"
              disabled={busy}
              onClick={onStop}
            >
              <Square size={14} aria-hidden /> {t("record.stop")}
            </Button>
          </div>
        ) : (
          <Button
            size="md"
            data-testid="btn-record"
            disabled={busy || blocked}
            onClick={onStart}
          >
            <Mic size={16} aria-hidden /> {t("record.start")}
          </Button>
        )}

        {/* The counterweight carries the gear now instead of being an empty box
            for nothing. Same width, same variant, right-aligned. */}
        <motion.div
          variants={side}
          className={clsx(
            "flex shrink-0 items-center justify-end overflow-hidden pl-2",
            recording && "hidden",
          )}
        >
          <Button
            variant="ghost"
            size="icon"
            data-testid="btn-dock-settings"
            onClick={onOpenSettings}
            title={t("nav.settings")}
            aria-label={t("nav.settings")}
          >
            <Settings size={16} />
          </Button>
        </motion.div>
      </motion.div>
    </motion.div>
  );
}

/**
 * One capture channel, as a button.
 *
 * The device name is never in the dock's box model. It lives in the tooltip, in
 * the accessible name and in the popover — so a 60-character
 * `Headset Microphone (Realtek(R) Audio) — 2 - Stereo Mix` cannot set the
 * dock's width, which is what a 110px button inside a 455px pill was. No colour
 * on the icons either: violet and cyan distinguish two things of the same kind
 * in the level meter and the transcript, and a coloured dot 8px from the
 * accent-filled Record button dilutes the one accent fill on the screen.
 *
 * Keyboard, outside click and focus are ModelPicker's handlers minus the search
 * box and the sections: ↑↓ clamped rather than wrapped, Enter commits, Escape
 * closes, a document-level `pointerdown` closes on an outside click. With no
 * search box to hold focus it stays on the trigger, which is why the trigger is
 * a `combobox` naming its active row rather than a plain button.
 */
function DeviceButton({
  kind,
  name,
  options,
  current,
  open,
  onOpenChange,
  onPick,
}: {
  kind: DeviceKind;
  name: string;
  options: DeviceOption[];
  current: string | null;
  open: boolean;
  onOpenChange: (kind: DeviceKind, open: boolean) => void;
  onPick: (kind: DeviceKind, id: string | null) => void;
}) {
  const { t } = useI18n();
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const listId = `dock-devices-${kind}`;
  const label = t(kind === "mic" ? "onboarding.mic" : "onboarding.system");

  useEffect(() => {
    if (!open) return;
    // Land on the current device rather than on row one: Enter straight after
    // opening would otherwise switch the capture device silently.
    setActive(Math.max(0, options.findIndex((o) => o.id === current)));
    // Deliberately keyed on `open` alone — re-running as the device list
    // refreshes would fight the arrow keys.
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onOpenChange(kind, false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open, kind, onOpenChange]);

  // Focus stays on the trigger, so the highlighted row has to be brought into
  // view by hand. `nearest` is a no-op when the row is already visible, which is
  // what keeps it from fighting the mouse.
  useEffect(() => {
    if (!open) return;
    listRef.current
      ?.querySelector<HTMLElement>("[data-active='true']")
      ?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  function commit(id: string | null) {
    // Writes immediately: there is no Save button within 400px of the dock, and
    // a choice that needs confirming somewhere else is not a choice made here.
    onPick(kind, id);
    onOpenChange(kind, false);
  }

  function onKeyDown(e: KeyboardEvent<HTMLButtonElement>) {
    // Closed, Enter and Space are the trigger's own activation and must stay
    // that way — this handler only owns the open list.
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      onOpenChange(kind, false);
      return;
    }
    if (e.key === "Enter") {
      // Also suppresses the click this keydown would otherwise synthesise,
      // which would reopen the list the commit just closed.
      e.preventDefault();
      const picked = options[active];
      if (picked) commit(picked.id);
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    // Clamped, not wrapped, matching ModelPicker: one idiom for a popover list.
    setActive((i) =>
      e.key === "ArrowDown"
        ? Math.min(options.length - 1, i + 1)
        : Math.max(0, i - 1),
    );
  }

  return (
    <div
      ref={rootRef}
      className="relative"
      onBlur={(e) => {
        // Tab out closes, but focus stays where the user sent it — pulling it
        // back to the trigger would trap Tab inside the dock.
        if (!e.currentTarget.contains(e.relatedTarget)) onOpenChange(kind, false);
      }}
    >
      <Button
        variant="ghost"
        size="icon"
        data-testid={`btn-dock-${kind}`}
        // `combobox` rather than a plain button: focus stays here while the list
        // is open, so the active row has to be named by `aria-activedescendant`,
        // and that attribute is only valid on a role that owns a popup.
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open ? `${listId}-opt-${active}` : undefined}
        // The name is the readout. The device name is deliberately outside the
        // dock's box model, so this is where it is legible at rest.
        title={`${label} — ${name}`}
        aria-label={`${label} — ${name}`}
        onKeyDown={onKeyDown}
        onClick={() => onOpenChange(kind, !open)}
      >
        {kind === "mic" ? <Mic size={16} /> : <MonitorSpeaker size={16} />}
      </Button>

      {/* Opens upward, because the dock is 24px from the bottom of the card.
          `max-h-64` with its own scroll because the card clips, and a machine
          with fifteen capture devices must not push this list through the
          card's top edge. */}
      {open && (
        <div
          ref={listRef}
          id={listId}
          role="listbox"
          aria-label={label}
          className="absolute bottom-full left-0 z-20 mb-2 max-h-64 min-w-56 max-w-80 overflow-y-auto rounded-lg border border-border bg-surface-3 p-1 shadow-occlude"
        >
          {options.map((o, i) => (
            <button
              key={o.id ?? "default"}
              id={`${listId}-opt-${i}`}
              type="button"
              role="option"
              aria-selected={o.id === current}
              data-active={i === active}
              // The list is driven by aria-activedescendant, so the rows must
              // not be tab stops of their own.
              tabIndex={-1}
              title={o.name}
              onMouseMove={() => setActive(i)}
              // Keeps focus on the trigger through the click, so picking with
              // the mouse does not drop focus onto the body when the list
              // unmounts. The click still fires.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => commit(o.id)}
              className={clsx(
                "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm",
                i === active && "bg-row-selected",
              )}
            >
              <Check
                size={14}
                aria-hidden
                className={clsx("shrink-0", o.id !== current && "opacity-0")}
              />
              {/* The popover absorbs the long name — and even here it truncates,
                  with the whole string in `title`. */}
              <span className="truncate">{o.name}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
