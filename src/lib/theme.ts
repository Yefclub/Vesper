export type Theme = "light" | "dark";

const KEY = "vesper-theme";

/** What is on screen right now, read from the attribute the app itself sets.
 *  The drawer's control falls back to this when the settings row does not carry
 *  a theme — otherwise a row saved by a backend that has no such field yet comes
 *  back without it and the control reads Light while the app is dark. */
export function currentTheme(): Theme {
  return document.documentElement.getAttribute("data-theme") === "dark"
    ? "dark"
    : "light";
}

/** SQLite is the source of truth; localStorage is a boot cache that exists
 *  only because `getSettings()` is an IPC round-trip that cannot beat first
 *  paint. If they disagree, SQLite wins on the next frame. If the cache is
 *  cleared, boot falls back to light — which is the correct default, so the
 *  failure mode is invisible. That is a real argument for light-as-default. */
export function applyTheme(next: Theme) {
  const root = document.documentElement;
  try {
    localStorage.setItem(KEY, next);
  } catch {
    /* best effort */
  }
  if (root.getAttribute("data-theme") === next) return; // boot cache already matched
  root.setAttribute("data-theme-switching", "");
  root.setAttribute("data-theme", next);
  // Two frames: one to apply the attribute, one to paint it. A single rAF
  // removes the guard before the new values are painted and the tail of the
  // change animates anyway.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => root.removeAttribute("data-theme-switching")),
  );
}
