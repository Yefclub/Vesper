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

## Compute backends

The `compute_backend` setting (`auto` · `cuda` · `vulkan` · `cpu`) is resolved
in `domain::backend` against two separate facts: what the binary was compiled
with, and what ggml reports the machine has. An explicit choice that cannot be
honoured falls to the CPU rather than to the other GPU — a silent substitution
is indistinguishable from the setting working.

The two engines do not reach the GPU the same way:

| | llama.cpp | whisper.cpp |
|---|---|---|
| CUDA | yes, loaded at runtime | **never** |
| Vulkan | yes | yes |
| Backend selection | `dynamic-backends` (`GGML_BACKEND_DL`) | linked in |

`whisper-rs-sys` emits `rustc-link-lib=cudart` — a load-time import — and
offers no way to pass CMake arguments, so a binary built with its `cuda`
feature does not start at all on a machine without the NVIDIA runtime. One
installer that works everywhere is worth more than CUDA for transcription;
CUDA's real advantage is prompt processing, which is llama's side.

`dynamic-backends` also brings `GGML_CPU_ALL_VARIANTS`: nine CPU backends from
SSE4.2 to AVX-512, picked at startup. The libraries are staged by `build.rs`
into `src-tauri/gpu-backends/` and shipped as bundle resources, which on
Windows puts them beside the executable — where ggml looks for them.

## Data

`%APPDATA%/Vesper` (or OS equivalent): `vesper.db`, `recordings/`, `models/`.

## Updates

Tauri updater plugin → GitHub Releases `latest.json` + signed artifacts for Windows / macOS / Linux bundle targets.
