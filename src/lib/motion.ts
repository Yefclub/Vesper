/**
 * Motion tokens.
 *
 * Every animation in the app reads its timing from here — including the CSS
 * ones. `--default-transition-duration` and `--default-transition-timing-function`
 * in styles/index.css mirror `duration.instant` and `ease.out`, so a bare
 * `transition-colors` utility runs the same curve as a framer spring. Before
 * that mirror existed, the CSS transitions in the app all ran Tailwind's
 * default 150ms cubic-bezier(0.4,0,0.2,1) — a duration and a curve that
 * appeared nowhere in this file.
 *
 * Any value changed here must be changed there too.
 */

/** Seconds. Anything slower than `panel` reads as the app being stuck. */
export const duration = {
  /** Hover, focus, press — must land before the user doubts the click. */
  instant: 0.12,
  /** The default: content swaps, list items, balloons. */
  base: 0.22,
  /** Panels and overlays, where a longer arc communicates depth. */
  panel: 0.32,

  /** Dismissal. Always faster than the matching arrival — a thing that
   *  takes as long to leave as it took to appear feels like the app is
   *  arguing with you. */
  exit: 0.14,
  panelExit: 0.2,

  /** A new item settling into a list, composed with blur. The only tier
   *  above 320ms, and the only one that is not a response to a click.
   *  Capped low because in Vesper this fires on every transcript segment
   *  during a live meeting, not once per navigation. */
  arrival: 0.42,
} as const;

/**
 * `out` decelerates hard: the element arrives quickly and settles, which is what
 * makes an interface feel responsive rather than merely animated.
 * `arrival` decelerates harder still — it has 420ms to spend and should spend
 * almost all of it in the last 20% of the travel.
 */
export const ease = {
  out: [0.22, 1, 0.36, 1],
  arrival: [0.16, 1, 0.3, 1],
  inOut: [0.65, 0, 0.35, 1],
} as const;

/** Pixels of travel. Small on purpose — long slides age badly on repeat viewing. */
export const distance = {
  sm: 6,
  md: 12,
} as const;

export const transition = {
  fast: { duration: duration.instant, ease: ease.out },
  base: { duration: duration.base, ease: ease.out },
  panel: { duration: duration.panel, ease: ease.out },
  exit: { duration: duration.exit, ease: ease.out },
  panelExit: { duration: duration.panelExit, ease: ease.out },
  arrival: { duration: duration.arrival, ease: ease.arrival },
} as const;

/**
 * Content arriving in place: fade with a short rise.
 *
 * Symmetric. It used to omit `exit` deliberately, so that under
 * `AnimatePresence mode="wait"` a pane could not block the next one from
 * mounting — but the transcript pane then added `exit={{ opacity: 0 }}` of its
 * own, which meant leaving Transcript cost a 220ms dead wait while leaving
 * Summary or Chat was instant. The same click felt different depending on
 * which tab you were on. One short exit on all three is the fix; at 140ms the
 * wait is below the threshold where it reads as lag.
 */
export const fadeRise = {
  initial: { opacity: 0, y: distance.sm },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, transition: transition.exit },
  transition: transition.base,
} as const;

/**
 * A new transcript segment landing during live capture. The composed
 * blur+translate+opacity, and the only place in the app that uses blur in
 * motion. Filter is not composite-only, so this is capped at the handful of
 * nodes appended per minute and must never be applied to a list that renders
 * in bulk.
 */
export const segmentArrive = {
  initial: { opacity: 0, y: distance.sm, filter: "blur(2px)" },
  animate: { opacity: 1, y: 0, filter: "blur(0px)" },
  transition: transition.arrival,
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
  exit: { opacity: 0, x: "8%", transition: transition.panelExit },
  transition: transition.panel,
} as const;

/** The dim behind a panel. Opacity only — nothing to move. */
export const backdropFade = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0, transition: transition.exit },
  transition: transition.base,
} as const;
