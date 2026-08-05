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

- **STT**: whisper.cpp via `whisper-rs`, over the `ggml-*.bin` weights in the catalogue + OpenRouter `/audio/transcriptions`
- **LLM**: llama.cpp via `llama-cpp-2`, over GGUF weights + OpenRouter chat completions (`reasoning` toggle) + any OpenAI-compatible server on loopback or a private range

With no weights downloaded, transcription is an error rather than a substitute — there is nothing honest to put in place of speech nobody transcribed. Summarize and chat fall back to a keyword extract of the transcript, which is labelled as one.

## Compute backends

The `compute_backend` setting (`auto` · `cuda` · `vulkan` · `cpu`) is resolved
in `domain::backend` against two separate facts: what the binary was compiled
with, and what ggml reports the machine has. Only `cpu` pins the processor: a
GPU that was asked for and cannot be reached falls through to the best one that
can, because the setting names a way to the card rather than an end in itself.
The log line and the settings panel both say what it resolved to, so the
substitution is not silent.

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

Shared libraries are not optional anywhere. whisper-rs-sys and llama-cpp-sys-2
each vendor their own ggml, and two static copies of those symbols is
`LNK2005` on MSVC and `rust-lld: error: duplicate symbol` on Linux — the link
fails on both. On Linux that leaves the loader with nothing to go on, since
Tauri installs resources under `/usr/lib/Vesper` while the executable lives in
`/usr/bin`, so `build.rs` adds an rpath of `$ORIGIN/../lib/Vesper` and the
runtime search looks there too.

**What the release actually carries today**: Vulkan on Windows and Linux, CPU
on macOS. `gpu-cuda` builds and is wired end to end, but nothing publishes it
— it needs the CUDA toolkit on the runner and the cuBLAS redistributables in
the installer, which is a decision about installer size, not a missing piece
of code.

## Data

`%APPDATA%/Vesper` (or OS equivalent): `vesper.db`, `recordings/`, `models/`.

## Updates

Tauri updater plugin → GitHub Releases `latest.json` + signed artifacts for Windows / macOS / Linux bundle targets.
