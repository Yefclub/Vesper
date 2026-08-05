# README screenshots

The four images beside this file are what the README shows. They go stale whenever the interface changes, and there is no automation catching that yet — see the note at the end.

## How they were taken

Not by hand, and not against a real meeting. The application draws its own window chrome, so a capture of the WebView is the whole window.

1. **Build with the front end embedded.** `cargo build --release` alone leaves the binary pointed at the Vite dev URL and it opens on a connection error:

   ```bash
   npx tauri build --no-bundle
   ```

2. **Move your own database aside.** `dirs::data_dir()` resolves through `SHGetKnownFolderPath` on Windows and ignores `%APPDATA%`, so the data directory cannot be redirected with an environment variable — the file has to be renamed. Record its hash first and check it after restoring; the screenshots are not worth somebody's meetings.

   ```bash
   sha256sum "$APPDATA/Vesper/vesper.db"
   mv "$APPDATA/Vesper/vesper.db" "$APPDATA/Vesper/vesper.db.backup"
   ```

3. **Launch with a debugging port and seed a demo profile.** The first launch creates an empty database; close the application, insert the demo meetings, and launch again.

   ```powershell
   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9555"
   ```

4. **Drive it over CDP and capture.** `Emulation.setDeviceMetricsOverride` at 1360×850 with `deviceScaleFactor: 2`, then `Page.captureScreenshot`, then downscale to 1360 wide.

   Click through the DOM, never at a screen coordinate — a synthetic click at a position lands wherever that position happens to be, which has already closed the wrong window once. And never click an element labelled *close*: on this window that is the title bar's own Close button.

5. **Restore, and verify the hash matches.**

## What is deliberately not automated

Rendering the application in CI and diffing the result against these files was considered and rejected. It needs a virtual display, a seeded profile, and a build — a job of roughly fifteen minutes — and it fails on antialiasing, font substitution and whichever animation frame the capture lands on. A screenshot test that is red for none of those reasons teaches everyone to refresh the baseline without looking at it, which is worse than no test.

What would work, and is not built yet, is a staleness guard: fail a pull request that changes the components in these shots without touching this directory. It cannot see pixels, so it will occasionally fire on a change that did not alter anything visible — the escape hatch is part of the design, not a flaw in it.
