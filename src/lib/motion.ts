/**
 * Motion tokens.
 *
 * Every animation in the app reads its timing from here. Scattered magic numbers
 * are what make an interface feel assembled by different hands: one panel eases
 * over 150ms, the next springs over 400, and the whole thing reads as noise
 * rather than as one system.
 */

/** Seconds. Anything slower than `slow` reads as the app being stuck. */
export const duration = {
  /** Hover, focus, press — must land before the user doubts the click. */
  instant: 0.12,
  /** The default: tab switches, list items, content swaps. */
  base: 0.22,
  /** Panels and overlays, where a longer arc communicates depth. */
  slow: 0.32,
} as const;

/**
 * `out` decelerates hard: the element arrives quickly and settles, which is what
 * makes an interface feel responsive rather than merely animated.
 */
export const ease = {
  out: [0.22, 1, 0.36, 1],
  inOut: [0.65, 0, 0.35, 1],
} as const;

/** Pixels of travel. Small on purpose — long slides age badly on repeat viewing. */
export const distance = {
  sm: 6,
  md: 12,
} as const;

export const transition = {
  base: { duration: duration.base, ease: ease.out },
  fast: { duration: duration.instant, ease: ease.out },
  panel: { duration: duration.slow, ease: ease.out },
} as const;

/**
 * Content arriving in place: fade with a short rise.
 *
 * Deliberately no `exit`. Inside `AnimatePresence mode="wait"` an exit blocks the
 * next panel from mounting until it finishes, so handing one to every pane would
 * put a fifth of a second of dead time on every tab click. Panes that should
 * animate out say so themselves.
 */
export const fadeRise = {
  initial: { opacity: 0, y: distance.sm },
  animate: { opacity: 1, y: 0 },
  transition: transition.base,
} as const;

/**
 * A panel anchored to the right edge, entering from it and leaving towards it.
 *
 * Spatial consistency: something that lives on the right should arrive from the
 * right, so the animation says where it came from instead of just announcing
 * that something appeared. Translation is a percentage of the panel, not a fixed
 * distance, so it clears its own width at any size.
 */
export const slideInRight = {
  initial: { opacity: 0, x: "8%" },
  animate: { opacity: 1, x: 0 },
  exit: { opacity: 0, x: "8%" },
  transition: transition.panel,
} as const;

/** The dim behind a panel. Opacity only — nothing to move. */
export const backdropFade = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0 },
  transition: transition.base,
} as const;
