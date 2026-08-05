/* Reads the boot cache before the bundle parses, so a dark-theme user does not
   get a frame of the light canvas.

   A real file with `<script src>`, never an inline `<script>`: the production
   CSP (`src-tauri/tauri.conf.json`) has no `script-src`, so `default-src 'self'`
   blocks inline scripts — while `devCsp` allows them. An inline boot script
   therefore works under `tauri dev` and fails silently in the shipped bundle.

   SQLite is the source of truth; this is only a cache, because `get_settings()`
   is an IPC round-trip that cannot beat first paint. If the cache is missing or
   unreadable the app boots light, which is the product default — so the failure
   mode is invisible. */
(function () {
  try {
    var t = localStorage.getItem("vesper-theme");
    if (t !== "dark" && t !== "light") return; // absent -> light default, correct
    document.documentElement.setAttribute("data-theme", t);
    // Not in the overlay document. An inline style beats every stylesheet, so
    // this line painted the floating card's window opaque in dark mode however
    // transparent the CSS said it was — a black rectangle behind the rounded
    // corners. `overlay.html` sets the class; this script runs in both.
    if (t === "dark" && !document.documentElement.classList.contains("overlay"))
      document.documentElement.style.background = "#0b090c";
  } catch (e) {
    /* storage blocked or cleared: light is the safe default */
  }
})();
