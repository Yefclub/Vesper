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

# Whatever a package decided to call its licence file. Matched as a prefix
# rather than a whole name: `@tauri-apps/api` ships `LICENSE_APACHE-2.0` and
# `LICENSE_MIT`, and an exact-match list skipped both while the table above went
# on listing the package.
LICENCE_PREFIXES = ("license", "licence", "copying", "notice")

# What an npm package may be under. The Rust half reads this from
# `src-tauri/about.toml`; npm has no equivalent, so it lives here. Same purpose:
# a dependency arriving under terms nobody read stops the build.
NPM_ALLOWED = {
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "0BSD",
    "CC0-1.0",
    "Unlicense",
    "Zlib",
}


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


def runtime_packages() -> list[str]:
    """Every package in the production graph, not only the ones we named.

    Vite bundles what the imports reach, and what they reach includes the
    dependencies of the dependencies — `framer-motion` brings `motion-dom` and
    `motion-utils`, `react-dom` brings `scheduler`. Reading only the direct
    entries in `package.json` misses code that is genuinely inside the
    installer, which is the failure this whole file exists to fix.

    `--omit=dev` is the other half of it: the build toolchain produces the
    bundle and is not in it, so listing it would claim an obligation this
    project does not have.
    """
    done = subprocess.run(
        ["npm", "ls", "--omit=dev", "--all", "--json"],
        cwd=ROOT,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        # npm is a shim on Windows and is not executable without one.
        shell=sys.platform == "win32",
    )
    # npm answers with the tree and a non-zero status for things this does not
    # care about — an extraneous package, a peer warning — so the output is what
    # matters rather than the code.
    if not done.stdout:
        sys.exit(done.stderr or "`npm ls` returned nothing — run `npm ci` first")
    names: set[str] = set()
    # Keyed on name and version, and only for nodes that HAVE children. npm
    # prints a package once per place it is required, and only one of those
    # copies carries its dependency list — `react-dom` appears empty beside the
    # other direct entries and full underneath. Skipping on the name alone lost
    # whichever copy came second, which is how `scheduler` went missing.
    walked: set[tuple[str, str]] = set()

    def walk(node: dict) -> None:
        for name, child in (node.get("dependencies") or {}).items():
            version = child.get("version")
            # No version means an unmet optional peer — npm lists it as an empty
            # object because something *could* use it. `framer-motion` asks for
            # `@emotion/is-prop-valid` this way. Not on disk, not in the bundle,
            # nothing to give notice of.
            if not version:
                continue
            names.add(name)
            if not (child.get("dependencies") or {}):
                continue
            key = (name, version)
            if key in walked:
                continue
            walked.add(key)
            walk(child)

    walk(json.loads(done.stdout))
    return sorted(names)


def npm() -> str:
    rows: list[str] = []
    texts: list[str] = []
    for name in runtime_packages():
        pkg_dir = ROOT / "node_modules" / Path(*name.split("/"))
        pkg_json = pkg_dir / "package.json"
        if not pkg_json.exists():
            sys.exit(f"{name} is not installed — run `npm ci` before this")
        meta = json.loads(pkg_json.read_text(encoding="utf-8"))
        licence = meta.get("license") or ""
        # The same allow-list the Rust half gets from `about.toml`, applied by
        # hand because npm has no equivalent. Crude on purpose: split the SPDX
        # expression and check every term, so `MIT OR GPL-3.0` fails on the half
        # that matters rather than passing on the half that does not.
        terms = [
            t.strip("() ")
            for t in licence.replace(" OR ", " ").replace(" AND ", " ").split()
        ]
        if not terms or any(t not in NPM_ALLOWED for t in terms):
            sys.exit(
                f"{name} is `{licence or 'unlicensed'}`, which is not on the "
                "npm allow-list in this script"
            )
        rows.append(f"| `{name}` | {meta.get('version', '?')} | {licence} |")
        found = [
            f
            for f in sorted(pkg_dir.iterdir())
            if f.is_file() and f.name.lower().startswith(LICENCE_PREFIXES)
        ]
        if not found:
            # Refused rather than skipped. A package listed in the table with no
            # text beneath it is the exact shape of the gap this file was
            # written to close, and it would go unnoticed.
            sys.exit(
                f"{name} ships no licence file — write its text into about.hbs by "
                "hand, the way the vendored C++ engines are handled"
            )
        # All of them. A dual-licensed package ships one file per licence, and
        # taking the first publishes half of what it granted.
        for f in found:
            texts.append(
                f"### {name} — {f.name}\n\n```\n"
                + f.read_text(encoding="utf-8", errors="replace").strip()
                + "\n```\n"
            )
    head = (
        "## npm packages\n\n"
        "The whole production graph, transitive dependencies included — Vite"
        " bundles what the imports reach. The build toolchain is not here: it"
        " produces the bundle and is not inside it.\n\n"
        "| package | version | licence |\n|---|---|---|\n"
    )
    return head + "\n".join(rows) + "\n\n" + "\n".join(texts)


def main() -> None:
    OUT.write_text(rust().rstrip() + "\n\n" + npm(), encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(ROOT)} ({OUT.stat().st_size // 1024} kB)")


if __name__ == "__main__":
    main()
