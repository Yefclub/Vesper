# Security policy

Vesper records meetings. A bug here is not an inconvenience — it is somebody's conversation.

## Reporting a vulnerability

**Do not open a public issue.** Use GitHub's private vulnerability reporting: go to the [Security tab](https://github.com/Yefclub/Vesper/security/advisories/new) and open a draft advisory. Only the maintainers see it.

Tell us what you found, how to reproduce it, and what an attacker gets out of it. A proof of concept helps; a video is not required.

You will get a first response within **7 days**. If a fix is warranted, we will agree a disclosure date with you before publishing, and you will be credited in the advisory unless you ask not to be.

## What is in scope

The parts of this application where a bug costs a user their privacy:

- **The IPC surface.** Every `#[tauri::command]` in `src-tauri/src/commands.rs` is reachable from the WebView. Anything that turns a parameter into a filesystem path, a network address, a download, or a SQL query is in scope.
- **Credential storage.** API keys go to the OS keychain (`src-tauri/src/secrets.rs`). A path that writes one somewhere else, logs it, or returns it in an error message is a vulnerability.
- **Egress.** The README lists every destination this application reaches and when. A path that carries audio, transcript text or notes anywhere not on that list is a vulnerability, as is a path that carries them to a listed destination in a situation the list does not cover. Offline mode failing to block something it says it blocks is the same bug — note that it deliberately does not block an OpenAI-compatible endpoint the user configured, which is documented rather than a finding.
- **Model downloads.** Origins come from an internal catalogue and every artifact is checked against a known digest before it is loaded (`src-tauri/src/models/mod.rs`). A way to make the app fetch or load something else belongs here.
- **The updater.** Release builds check a signed manifest. Anything that would let an unsigned or substituted artifact install is in scope.

## What is not

- Vulnerabilities in the operating system, the WebView runtime, or a model you downloaded yourself.
- Attacks that require an attacker who already has code execution as your user account. On a desktop application that account boundary is the boundary; an attacker inside it can read the database directly.
- Findings from automated scanners with no demonstrated impact.

## Supported versions

The latest release. Vesper is early: fixes go into the next release rather than being backported.
