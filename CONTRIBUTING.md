# Contributing to Vesper

Thanks for looking. This file is what you need to get from a clone to a merged pull request.

## Before you build

See [the README](README.md#develop) for prerequisites. Two of them bite on first build and are worth repeating:

- **Windows**: LLVM must be on `PATH`, not just `LIBCLANG_PATH`. The build script asks for `nm` and then, once it finds that, for `objcopy` — resolving them one variable at a time costs you a build round per tool.
- **Linux**: the five apt packages in the README are not on Tauri's prerequisites page, and without them the build script fails rather than the compiler.

```bash
npm install
npm run tauri dev
```

## Before you open a pull request

Run these, in this order, and paste the output in the pull request:

```bash
npm run typecheck
npm run build
cd src-tauri && cargo test
```

CI additionally runs `cargo fmt --check` and `cargo clippy -- -D warnings -A dead_code` on Linux and Windows. Run them locally and you will not learn about them twenty minutes later.

Red at any step means the pull request is not ready. We would rather wait.

## How changes are shaped

- **The smallest change that solves the problem.** No feature beyond the one asked for, no abstraction for a single use, no handling of an error that cannot happen.
- **Surgical.** Do not improve neighbouring code, do not refactor what is not broken, and imitate the surrounding style even where you disagree with it. An orphan your own change created — an import, a variable — goes with it. Pre-existing dead code gets pointed at, not deleted.
- **Every changed line traces to the request.** If it does not, it comes out of the diff.
- **Ambiguity is a question, not a guess.** Ask in the issue before writing the code.

Commits follow [conventional commits](https://www.conventionalcommits.org/) and are written in English, as are pull request titles and bodies. Branch names are `type/short-description`, with `type` one of `feat`, `fix`, `chore`, `docs`, `ci`, `refactor`.

Pull requests target **`dev`**. `main` is the release branch and only receives promotions.

## Where the tests live

`src-tauri/src/domain/` is pure logic and is where the tests are. If your change can be expressed as a function over data, put it there and test it there — the ports in `audio/`, `stt/`, `llm/` and `db/` are thin on purpose.

There is no front-end test runner. `npm run lint` is an alias of `npm run typecheck`; do not read a green lint as style coverage.

## Things that need a conversation first

Open an issue before writing code if your change:

- **Sends anything new over the network.** This application contacts three hosts and only when the user asks. A fourth is a product decision, not a patch.
- **Changes the database schema destructively.** The database is the user's own file, at `%APPDATA%/Vesper/vesper.db` or the equivalent. There is no environment where it can just be recreated: a dropped column is somebody's meeting. Additive migrations in `db::migrate()` are fine.
- **Adds a dependency.** Everything you add ships inside a desktop installer. Check whether the project already has something equivalent.
- **Touches the release workflow or the signing configuration.**

## Reporting a security problem

Not here — see [SECURITY.md](.github/SECURITY.md). Please do not open a public issue for a vulnerability.

## Agents

Several of the instruction files in this repository are written for coding agents (`AGENTS.md`, `CLAUDE.md`, `.claude/`). They are the same rules as this document, in the form a tool reads. If you are using an agent, point it at `AGENTS.md`.
