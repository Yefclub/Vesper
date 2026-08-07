"""Write the updater manifest from the assets a release actually holds.

`tauri-action` writes `latest.json` from inside each platform's build job, and
three jobs writing one file is a race nobody wins reliably. On the `dev-10`
build the Windows job finished last — 01:39:39 against the Linux job's 01:32:23
— and the manifest that survived was the Linux one, still naming the Windows
installer from two builds earlier. Every Windows install then offered an update,
downloaded the version it already had, restarted unchanged, and offered it
again.

So the manifest is not written by a build. It is written once, after all of them,
from what is in the release: name the version, read the assets, pair each
installer with its signature. A platform whose artifact is missing is an error
rather than an entry quietly carried over from last time, which is the failure
this replaces.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# Tauri reads the base key; the suffixed ones let an install pick a specific
# bundle. Both shapes are what `tauri-action` emitted, and dropping either would
# be a change to how existing installs resolve an update rather than a tidy-up.
#
# Order matters within a platform: the first match wins the base key, so the
# bundle listed first is the one an ordinary update installs.
WANTED: dict[str, list[tuple[str, str]]] = {
    "darwin-aarch64": [("app", ".app.tar.gz")],
    "windows-x86_64": [("nsis", "-setup.exe"), ("msi", ".msi")],
    "linux-x86_64": [("appimage", ".AppImage"), ("deb", ".deb"), ("rpm", ".rpm")],
}


def gh(*args: str) -> str:
    done = subprocess.run(
        ["gh", *args], capture_output=True, encoding="utf-8", errors="replace"
    )
    if done.returncode != 0:
        sys.exit(done.stderr.strip() or f"gh {' '.join(args)} failed")
    return done.stdout


def assets(tag: str) -> list[dict]:
    return json.loads(gh("release", "view", tag, "--json", "assets"))["assets"]


def signature(tag: str, name: str, work: Path) -> str:
    """The detached signature, as the manifest carries it.

    Downloaded rather than read from the build tree: this runs in a job that
    built nothing, which is the whole point — it sees what the release holds
    rather than what one runner happened to produce.
    """
    out = work / name
    if not out.exists():
        gh("release", "download", tag, "--pattern", name, "--dir", str(work))
    return out.read_text(encoding="utf-8").strip()


def build(
    tag: str, version: str, notes: str, pub_date: str, built_after: str, work: Path
) -> dict:
    have = {a["name"]: a for a in assets(tag)}
    # Only this version's artifacts. A release whose assets are replaced in
    # place — the dev channel is one — still holds every previous build, and
    # matching on the extension alone would pick whichever sorted first.
    #
    # Bounded on both sides, because a bare `in` is wrong in a way that reads
    # fine: `0.3.0-1` is inside `0.3.0-10`, so re-running an old build after a
    # newer one would produce a manifest labelled `0.3.0-1` pointing at the
    # `-10` installer. Every bundler here puts a separator either side.
    token = re.compile(rf"[_\-.]{re.escape(version)}[_\-.]")
    mine = {n: a for n, a in have.items() if token.search(n)}
    if not mine:
        sys.exit(f"{tag} holds no artifact naming version {version}")
    platforms: dict[str, dict] = {}
    for base, bundles in WANTED.items():
        for suffix, ext in bundles:
            # macOS is the exception and has to be handled rather than filtered
            # out: `Vesper_aarch64.app.tar.gz` carries no version, so there is
            # one of them and each build overwrites it.
            versioned = any(token.search(n) for n in have if n.endswith(ext))
            pool = mine if versioned else have
            artifact = next(
                (n for n in sorted(pool) if n.endswith(ext) and not n.endswith(".sig")),
                None,
            )
            if artifact is None:
                continue
            # An unversioned name is only this build's if this build wrote it.
            # Compared against when the run started rather than against the other
            # artifacts: the jobs finish in whatever order the runners allow, and
            # on the build this was written for macOS finished first — so a floor
            # taken from its siblings would have rejected a bundle that was
            # perfectly current.
            if not versioned and have[artifact]["updatedAt"] < built_after:
                sys.exit(
                    f"{artifact} predates this run ({have[artifact]['updatedAt']} < "
                    f"{built_after}) — its build did not replace it, and the manifest "
                    "would ship the previous one"
                )
            sig_name = artifact + ".sig"
            if sig_name not in have:
                # Unsigned means no existing install will accept it. Naming the
                # platform is more use than a manifest that silently omits it.
                sys.exit(f"{artifact} has no signature — {base} cannot be published")
            entry = {
                "signature": signature(tag, sig_name, work),
                "url": have[artifact]["url"],
            }
            platforms.setdefault(base, entry)
            platforms[f"{base}-{suffix}"] = entry
    missing = [b for b in WANTED if b not in platforms]
    if missing:
        sys.exit(
            f"no artifact for {', '.join(missing)} in {tag} at version {version} — "
            "the manifest would carry the previous build for those platforms"
        )
    return {
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": platforms,
    }


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--tag", required=True, help="release the assets live in")
    p.add_argument("--version", required=True, help="version string the manifest declares")
    p.add_argument("--notes", default="")
    p.add_argument("--pub-date", required=True, help="RFC 3339, from the workflow")
    p.add_argument(
        "--built-after",
        required=True,
        help="RFC 3339 start of the run; an unversioned artifact older than this "
        "belongs to a previous build",
    )
    p.add_argument("--work", default=".manifest", help="where signatures are downloaded")
    a = p.parse_args()

    work = Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    manifest = build(a.tag, a.version, a.notes, a.pub_date, a.built_after, work)

    out = work / "latest.json"
    out.write_text(json.dumps(manifest, indent=2), encoding="utf-8", newline="\n")
    # `--clobber`, because the build jobs have already left one there.
    gh("release", "upload", a.tag, str(out), "--clobber")
    print(f"latest.json -> {a.version}")
    for k in sorted(manifest["platforms"]):
        print(f"  {k}: {manifest['platforms'][k]['url'].rsplit('/', 1)[-1]}")


if __name__ == "__main__":
    main()
