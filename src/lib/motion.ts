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
 * Content arriving in place: fade with a short rise. Exit only fades — reversing
 * the travel on the way out draws the eye to something that is leaving.
 */
export const fadeRise = {
  initial: { opacity: 0, y: distance.sm },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0 },
  transition: transition.base,
} as const;
