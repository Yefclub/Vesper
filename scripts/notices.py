"""Rebuild THIRD-PARTY-NOTICES.md from both lockfiles.

Two package managers ship into one installer, so one file has to cover both.
`cargo about` does the Rust half against `src-tauri/about.toml`; the npm half is
walked here, because the runtime tree is fourteen packages and a second tool for
that is more to keep working than to read.

Run it with `npm run notices`. It fails rather than writing a partial file: a
notices document that is quietly missing a dependency is worse than one that is
obviously out of date.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "src-tauri"
OUT = ROOT / "THIRD-PARTY-NOTICES.md"

# The licence file each package ships under, whatever it decided to call it.
LICENCE_NAMES = ("LICENSE", "LICENCE", "license", "licence", "COPYING")


def rust() -> str:
    if not shutil.which("cargo-about"):
        sys.exit(
            "cargo-about is not installed. `cargo install cargo-about --locked --features cli`"
        )
    done = subprocess.run(
        ["cargo", "about", "generate", "about.hbs"],
        cwd=TAURI,
        capture_output=True,
        # Named, not left to the platform. Python decodes a child's output with
        # the ANSI codepage on Windows, and a copyright line with an accent in
        # it — there are several — kills the reader thread rather than the run,
        # so the output comes back empty with a zero exit code.
        encoding="utf-8",
        errors="replace",
    )
    if done.returncode != 0:
        # The interesting case is a dependency whose licence is not on the
        # allow-list: cargo-about names it on stderr, and that message is the
        # whole point of running this in CI.
        sys.exit(done.stderr or "cargo about failed with no message")
    return done.stdout


def npm() -> str:
    """The runtime tree only.

    `devDependencies` build the bundle and do not enter it, so listing them
    would claim an obligation this project does not have.
    """
    manifest = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
    rows: list[str] = []
    texts: list[str] = []
    for name in sorted(manifest.get("dependencies", {})):
        pkg_dir = ROOT / "node_modules" / Path(*name.split("/"))
        pkg_json = pkg_dir / "package.json"
        if not pkg_json.exists():
            sys.exit(f"{name} is not installed — run `npm ci` before this")
        meta = json.loads(pkg_json.read_text(encoding="utf-8"))
        licence = meta.get("license") or "?"
        rows.append(f"| `{name}` | {meta.get('version', '?')} | {licence} |")
        for candidate in pkg_dir.iterdir():
            if candidate.is_file() and candidate.name.split(".")[0] in LICENCE_NAMES:
                texts.append(
                    f"### {name}\n\n```\n"
                    + candidate.read_text(encoding="utf-8", errors="replace").strip()
                    + "\n```\n"
                )
                break
    head = (
        "## npm packages\n\n"
        "Only what the application loads at runtime. The build toolchain —"
        " TypeScript, Vite, Tailwind — produces the bundle and is not inside"
        " it.\n\n"
        "| package | version | licence |\n|---|---|---|\n"
    )
    return head + "\n".join(rows) + "\n\n" + "\n".join(texts)


def main() -> None:
    OUT.write_text(rust().rstrip() + "\n\n" + npm(), encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(ROOT)} ({OUT.stat().st_size // 1024} kB)")


if __name__ == "__main__":
    main()
