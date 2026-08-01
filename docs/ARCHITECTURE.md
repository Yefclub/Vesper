# Vesper architecture

## Process model

Single Tauri process:

- **Rust**: dual-channel capture, STT, LLM, SQLite, exports, tray/hotkeys/updater
- **WebView**: React UI via Tauri commands + events

No Python sidecar, no Docker for end users.

## Dual-channel audio

| Channel | Source | Label |
|---------|--------|-------|
| 0 | Microphone | Me |
| 1 | System / loopback (or stereo R) | Others |

PCM is written as stereo WAV (L=Me, R=Others). Live STT runs per channel and merges into an ordered transcript.

## Jobs / meeting states

`idle → recording ⇄ paused → transcribing → ready ⇄ summarizing`

Failures go to `failed` with reset back to `idle`.

## Providers

- **STT**: local ONNX/Moonshine/Parakeet-class path + OpenRouter `/audio/transcriptions`
- **LLM**: local GGUF path + OpenRouter chat completions (`reasoning` toggle)

Offline fallbacks keep summarize/chat usable without downloaded weights.

## Data

`%APPDATA%/Vesper` (or OS equivalent): `vesper.db`, `recordings/`, `models/`.

## Updates

Tauri updater plugin → GitHub Releases `latest.json` + signed artifacts for Windows / macOS / Linux bundle targets.
