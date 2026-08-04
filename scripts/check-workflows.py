#!/usr/bin/env python3
"""Catch the packaging failures that only ever showed up in CI.

Six dev builds were spent discovering, one at a time, faults that were all
visible in the workflow files the whole time. Each cost a full matrix run to
learn. Every check here is a rule that a real failure taught, named after what
it broke, so a rule that stops earning its place can be deleted knowing what is
being given up.

Run: python scripts/check-workflows.py
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

# Windows consoles still default to cp1252, and a report is worthless if
# printing it raises. This script found its own first bug that way: the MSI rule
# fired correctly and then died on the arrow in its own message, which read from
# the outside exactly like the rule not firing at all.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

WORKFLOWS = pathlib.Path(".github/workflows")
TAURI_CONF = pathlib.Path("src-tauri/tauri.conf.json")

failures: list[str] = []


def fail(check: str, where: str, detail: str) -> None:
    failures.append(f"{check}\n    {where}\n    {detail}")


def read(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8")


# --- 1. YAML that parses, with duplicate keys refused -----------------------
#
# A second `APPIMAGE_EXTRACT_AND_RUN` under the same `env:` was written by
# accident and YAML accepts it silently, last one winning. Nothing downstream
# would have complained.
def check_parses(paths: list[pathlib.Path]) -> None:
    try:
        import yaml
    except ImportError:
        # Not a skip. AGENTS.md requires this script before a workflow change,
        # and a required check that quietly does not run is worse than one that
        # is missing: the exit code says the workflows were checked.
        fail(
            "the duplicate-key check cannot run",
            "pyyaml is not installed",
            "pip install pyyaml -- without it this rule passes everything, "
            "including the duplicated env key that shipped once already",
        )
        return

    class NoDuplicates(yaml.SafeLoader):
        pass

    def no_duplicate_keys(loader, node, deep=False):
        seen = set()
        for key_node, _ in node.value:
            key = loader.construct_object(key_node, deep=deep)
            if key in seen:
                raise yaml.constructor.ConstructorError(
                    None, None, f"duplicate key {key!r}", key_node.start_mark
                )
            seen.add(key)
        return yaml.constructor.SafeConstructor.construct_mapping(loader, node, deep)

    NoDuplicates.add_constructor(
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, no_duplicate_keys
    )

    for path in paths:
        try:
            yaml.load(read(path), Loader=NoDuplicates)
        except yaml.YAMLError as e:
            fail("workflow does not parse", str(path), str(e).replace("\n", " "))


# --- 2. A version the MSI bundler will accept -------------------------------
#
# `0.1.0-dev.1` was refused with "pre-release identifier must be numeric-only",
# after NSIS had already bundled. The dev channel builds its version by
# appending to the one in tauri.conf.json, so the rule is checked against what
# that expression actually produces.
def check_msi_version() -> None:
    if not TAURI_CONF.is_file():
        return
    base = json.loads(read(TAURI_CONF)).get("version", "")

    suffixes = set()
    for path in WORKFLOWS.glob("*.yml"):
        # The closing quote has to be the same one that opened. The f-string
        # interpolates `base["version"]`, so it contains the other quote
        # character — matching either one non-greedily captured `{base[` and
        # then found nothing to check. The first version of this rule passed
        # cleanly on the exact expression that broke the build, which is why it
        # is now tested against that commit rather than against today's tree.
        for m in re.finditer(
            r'base\["version"\]\s*=\s*f?([\'"])(.+?)\1', read(path)
        ):
            suffixes.add((path, m.group(2)))

    for path, expr in suffixes:
        # `{base["version"]}-{...}` with the interpolations reduced to a sample
        # value, which is all the identifier rules need to be applied to.
        rendered = expr.replace('{base["version"]}', base)
        rendered = re.sub(r"\{[^}]+\}", "7", rendered)
        if "-" not in rendered:
            continue
        pre = rendered.split("-", 1)[1]
        for identifier in pre.split("."):
            if not identifier.isdigit():
                fail(
                    "MSI refuses this version",
                    f"{path} builds {rendered}",
                    f"pre-release identifier {identifier!r} is not numeric-only; "
                    "the msi bundler rejects the build after nsis has succeeded",
                )
            elif int(identifier) > 65535:
                fail(
                    "MSI refuses this version",
                    f"{path} builds {rendered}",
                    f"pre-release identifier {identifier} is above 65535",
                )


# --- 3. Distro codenames that follow the runner -----------------------------
#
# Pinning the Linux job to 22.04 left the LunarG apt line asking for the noble
# list, and apt offered packages built against a glibc the image does not have.
# Sixteen unmet dependencies, none of which said "Vulkan".
CODENAMES = ("focal", "jammy", "noble", "plucky", "questing")


def check_codenames(paths: list[pathlib.Path]) -> None:
    for path in paths:
        for n, line in enumerate(read(path).splitlines(), 1):
            if line.lstrip().startswith("#"):
                continue
            for codename in CODENAMES:
                if codename in line and "lsb_release" not in line:
                    fail(
                        "distro codename written down instead of derived",
                        f"{path}:{n}",
                        f"{codename!r} is pinned here but the runner is chosen "
                        "elsewhere; use $(lsb_release -cs) so the two cannot drift",
                    )


# --- 4. The AppImage bundler has to be told where our libraries are ---------
#
# This project dynamic-links its own ggml, llama and whisper — two vendored
# copies of ggml collide at link time otherwise — so the binary depends on
# libllama.so.0, which lives in a resource directory. linuxdeploy walks the
# ELF's dependencies and stops at the first it cannot resolve.
def check_appimage_libs(paths: list[pathlib.Path]) -> None:
    for path in paths:
        text = read(path)
        if "tauri-action" not in text or "ubuntu" not in text:
            continue
        if "LD_LIBRARY_PATH" not in text:
            fail(
                "AppImage bundling will not find our shared libraries",
                str(path),
                "this workflow bundles on Linux but never sets LD_LIBRARY_PATH; "
                "linuxdeploy fails with 'Could not find dependency: libllama.so.0'",
            )


# --- the rules have to be tested too -----------------------------------------
#
# Two of these rules shipped broken within an hour of being written: one matched
# the wrong quote and captured nothing, the other fired correctly and then died
# printing its own message. Both looked, from the outside, exactly like a clean
# run. A guard that fails open is worse than no guard, because it is trusted.
#
# Each sample is the shape that actually broke a build, reduced to the few lines
# the rule reads.
SAMPLES: list[tuple[str, str, dict[str, str]]] = [
    (
        "msi",
        "MSI refuses this version",
        {
            "src-tauri/tauri.conf.json": '{"version": "0.1.0"}',
            ".github/workflows/w.yml":
                'run: |\n  base["version"] = f\'{base["version"]}-dev.{n}\'\n',
        },
    ),
    (
        "codename",
        "distro codename written down",
        {
            ".github/workflows/w.yml":
                "runs-on: ubuntu-22.04\nrun: wget lunarg-vulkan-noble.list\n",
        },
    ),
    (
        "appimage",
        "AppImage bundling will not find",
        {
            ".github/workflows/w.yml":
                "runs-on: ubuntu-latest\nsteps:\n  - uses: tauri-apps/tauri-action@v0\n",
        },
    ),
    (
        "duplicate key",
        "workflow does not parse",
        {".github/workflows/w.yml": "env:\n  A: 1\n  A: 2\n"},
    ),
]


def self_test() -> int:
    import tempfile

    global WORKFLOWS, TAURI_CONF
    silent = []
    for name, expected, files in SAMPLES:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for rel, body in files.items():
                target = root / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(body, encoding="utf-8")
            WORKFLOWS = root / ".github" / "workflows"
            TAURI_CONF = root / "src-tauri" / "tauri.conf.json"

            failures.clear()
            paths = sorted(WORKFLOWS.glob("*.yml"))
            check_parses(paths)
            check_msi_version()
            check_codenames(paths)
            check_appimage_libs(paths)

            # By the rule that had to fire, not by "something failed". Merely
            # counting let a sample pass on the back of an unrelated rule: with
            # pyyaml absent every sample reported a missing parser and the whole
            # self-test went green without exercising anything.
            hit = next((f for f in failures if expected in f), None)
            if hit is None:
                silent.append(name)
                continue
            # Printed, not merely matched. The second bug in this file was in
            # the printing, and a check that never formats its own report would
            # have passed while the real one crashed.
            print("  {}: caught -- {}".format(name, hit.splitlines()[0]))

    if silent:
        print("\nthe guard itself is broken:\n")
        for name in silent:
            print("  {}: rule did not fire on the shape that broke a build".format(name))
        return 1
    print("self-test: {} known failures, all caught".format(len(SAMPLES)))
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not WORKFLOWS.is_dir():
        print("no .github/workflows - run this from the repository root", file=sys.stderr)
        return 2
    paths = sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))

    check_parses(paths)
    check_msi_version()
    check_codenames(paths)
    check_appimage_libs(paths)

    if failures:
        print("\n{} problem(s) that would have cost a CI run:\n".format(len(failures)))
        for f in failures:
            print("  {}\n".format(f))
        return 1
    print("workflows: {} checked, nothing that has broken a build before".format(len(paths)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
