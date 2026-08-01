# Vesper — project rules

## Product

Desktop privacy-first AI meeting note-taker (Tauri 2 + React). Local-first STT/LLM; OpenRouter optional. No accounts, no telemetry.

## Branches

- **`dev`**: integration branch — all feature PRs target `dev`
- **`main`**: release branch — version tags + updater artifacts

Never commit directly to `main` or `dev` without PR (except initial bootstrap).

## Commands

```bash
npm install
npm run tauri dev
npm run typecheck
npm run build
cd src-tauri && cargo test
npm run tauri build
```

## Layout

- `src/` — React UI (Grok Night dark theme)
- `src-tauri/src/domain/` — pure logic (tests live here)
- `src-tauri/src/audio|stt|llm|db/` — ports + implementations
- `docs/` — tracked product notes

## Hygiene

- Ignore models, recordings, `.env`, `CLAUDE.local.md`, `SPEC.md`
- No competitor names in user-facing strings or README marketing
- MIT license; open-source ready

## Language

Code, commits, and PR titles in English. Chat with maintainer may be PT-BR.
