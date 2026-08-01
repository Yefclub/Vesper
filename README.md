# Vesper

Privacy-first desktop AI meeting note-taker. Capture **microphone + system audio** as separate channels, live-transcribe with **Me / Others** labels, summarize and chat over meetings — all **local by default**. Optional OpenRouter for cloud STT/LLM.

## Features

- One-click record / pause / resume / stop
- Dual-channel capture (mic = Me, system = Others) — no bot joins your call
- Live streaming transcript + file import / retranscription
- Local ultra-light STT & LLM (download on first use) + optional OpenRouter
- Summary templates, action items, key points, meeting chat
- SQLite + local files only — no account, no telemetry
- Search across meetings; export Markdown / PDF / DOCX
- System tray, global hotkey (`Ctrl/Cmd+Shift+R`), automatic update checks
- Installers for **Windows**, **macOS**, and **Linux**

## Stack

- **Desktop**: Tauri 2 + Rust
- **UI**: React 19, Vite, TypeScript, Tailwind CSS 4, Framer Motion
- **Data**: SQLite (rusqlite), local audio files

## Develop

Prerequisites: [Node.js](https://nodejs.org/) 20+, [Rust](https://rustup.rs/), platform WebView deps ([Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).

**Native ML build** (local Whisper + llama.cpp): install [LLVM](https://github.com/llvm/llvm-project/releases) so `libclang` is available. On Windows:

```powershell
winget install -e --id LLVM.LLVM
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
$env:Path = "C:\Program Files\LLVM\bin;" + $env:Path
```

```bash
npm install
npm run tauri dev
```

Download Whisper / GGUF weights from **Settings** (Hugging Face direct links). Until models are installed, OpenRouter remains available; local engines require real weights (no fake transcription theater).

### Useful commands

```bash
# Frontend typecheck / production UI build
npm run typecheck
npm run build

# Rust unit tests (domain, STT pipeline, DB, export)
cd src-tauri && cargo test

# Desktop debug / release package
npm run tauri build
```

## Branches

| Branch | Role |
|--------|------|
| `dev`  | Integration — feature PRs land here |
| `main` | Release — tagged installs & auto-update artifacts |

## Settings

- **STT / LLM provider**: Local (default) or OpenRouter
- **Reasoning toggle**: CoT-style flag for OpenRouter models
- **API keys**: stored only on device in the local SQLite settings store

## Auto-update

Release builds check:

`https://github.com/Yefclub/Vesper/releases/latest/download/latest.json`

Signing public key is configured in `src-tauri/tauri.conf.json` (`plugins.updater`). Replace the placeholder pubkey before publishing signed releases.

## License

MIT — see [LICENSE](./LICENSE).
