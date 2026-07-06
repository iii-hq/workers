#!/usr/bin/env python3
"""Per-worker pr-checks validation.

Enforces:
    1. README.md exists and is non-empty.
    2. iii.worker.yaml parses and has required fields + valid enum values.
    3. The manifest version on this ref is greater than or equal to on --base-ref.
    4. tests/ exists and is non-empty.
    5. For workers in BOOTSTRAP_WORKERS, skills/SKILL.md exists, is non-empty,
       and is within the 256 KiB cap — the harness bootstraps these onto disk via
       iii-directory on first boot; a missing or oversized file breaks the
       chat surface's orientation.

If `--worker` is not in `--source-changed`, requirements 1, 3, and 4 are
downgraded to GitHub Actions notices instead of hard errors. Requirement
5 is always strict — it's a release-blocking guarantee, not a hygiene
check.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402


# Workers whose skills the harness stack requires at boot, making
# skills/SKILL.md a hard PR gate. Keep in sync with what the harness
# actually bootstraps.
BOOTSTRAP_WORKERS = frozenset({
    "iii-directory",
    "shell",
})

SKILL_MD_SIZE_CAP = 256 * 1024  # 256 KiB

# `iii worker add <worker>` downloads the release archive and looks for a
# binary named after the WORKER (see iii crates/iii-worker binary_download.rs:
# extract_binary_from_targz(worker_name, ...)). The registry payload carries
# only the worker name, never the cargo [[bin]] name, so the packaged binary
# MUST be named after the worker or the resolver fails with
# "Binary '<worker>' not found in archive" (web/v1.1.x shipped this bug).
# Workers below intentionally ship a differently-named, user-facing binary and
# are known-broken on fresh `iii worker add` until the resolver carries the
# bin name through the registry; do not silently extend this list.
BINARY_NAME_EXCEPTIONS = frozenset({
    "acp",  # editors launch the `iii-acp` binary by name (ACP subprocess)
})

# The engine's bundle validator (iii-worker/src/cli/bundle_download.rs) only
# accepts runtime.base_image values that name a sandbox-catalog preset ref
# verbatim (sandbox_daemon/catalog.rs PRESETS); that's how a non-node bundle
# picks its rootfs now that runtime.kind is deprecated.
BUNDLE_PRESET_IMAGES = frozenset({
    "docker.io/iiidev/python:latest",
    "docker.io/iiidev/node:latest",
})


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--worker", required=True, help="worker folder name")
    p.add_argument("--base-ref", required=True, help="base branch ref, e.g. 'main'")
    p.add_argument(
        "--source-changed", required=True,
        help="JSON array of workers that had non-metadata source changes",
    )
    args = p.parse_args(argv)

    worker = args.worker
    source_changed = set(json.loads(args.source_changed))
    strict = worker in source_changed
    root = pathlib.Path(worker)
    errs: list[str] = []

    def hard(msg: str) -> None:
        errs.append(msg)

    def soft(msg: str) -> None:
        if strict:
            errs.append(msg)
        else:
            print(f"::notice::{msg} (skipped: {worker} only changed metadata)")

    # 1. README.md present and non-empty
    readme = root / "README.md"
    if not readme.exists():
        soft(f"{worker}/README.md is missing")
    elif readme.stat().st_size == 0:
        soft(f"{worker}/README.md is empty")

    # 2. iii.worker.yaml — always strict
    m = None
    try:
        m = _lib.read_iii_worker_yaml(root)
    except FileNotFoundError:
        hard(f"{worker}/iii.worker.yaml is missing")
    except ValueError as e:
        hard(f"{worker}/iii.worker.yaml: {e}")

    if m is not None:
        # Use raw dict, not WorkerManifest attrs: _lib silently fills `name`
        # with the folder name when the yaml omits it, so getattr(m, "name")
        # would hide a missing key.
        for key in ("name", "language", "deploy", "manifest"):
            if not m.raw.get(key):
                hard(f"{worker}/iii.worker.yaml is missing key: {key}")
        if m.name != worker:
            hard(f"{worker}/iii.worker.yaml name={m.name!r} does not match folder")
        if m.deploy not in ("binary", "image", "bundle"):
            hard(f"{worker}/iii.worker.yaml deploy must be 'binary', 'image', or 'bundle'")
        if m.language not in ("rust", "node", "python", "javascript"):
            hard(
                f"{worker}/iii.worker.yaml language must be 'rust' | 'node' | 'python' | 'javascript'"
            )
        # The release archive's binary is named after `bin` (defaulting to the
        # worker name); the resolver looks it up by worker name. They must match
        # or `iii worker add {worker}` fails with "Binary not found in archive".
        if m.deploy == "binary":
            effective_bin = m.bin or m.name
            if effective_bin != worker and worker not in BINARY_NAME_EXCEPTIONS:
                hard(
                    f"{worker}/iii.worker.yaml bin={effective_bin!r} must equal the "
                    f"worker name {worker!r} for binary deploys: `iii worker add` "
                    f"extracts the release archive by worker name and would fail "
                    f"with \"Binary '{worker}' not found in archive\""
                )
        # Mirror the engine's bundle-manifest validator
        # (iii-worker/src/cli/bundle_download.rs): it executes only
        # scripts.start and rejects install/setup/base_image at install
        # time. Catch that at PR time instead of at the user's install.
        if m.deploy == "bundle":
            scripts = m.raw.get("scripts") or {}
            if str(scripts.get("setup") or "").strip():
                hard(f"{worker}/iii.worker.yaml: bundle workers must not declare scripts.setup (engine rejects it)")
            if str(scripts.get("install") or "").strip():
                hard(f"{worker}/iii.worker.yaml: bundle workers must not declare scripts.install (engine rejects it)")
            if not str(scripts.get("start") or "").strip():
                hard(f"{worker}/iii.worker.yaml: bundle workers must declare a non-empty scripts.start")
            base_image = (m.raw.get("runtime") or {}).get("base_image")
            if base_image is not None and base_image not in BUNDLE_PRESET_IMAGES:
                hard(
                    f"{worker}/iii.worker.yaml: bundle runtime.base_image must be one of "
                    f"{sorted(BUNDLE_PRESET_IMAGES)} (engine rejects anything else)"
                )

    # 3. Manifest version >= base
    if m is not None and m.manifest:
        manifest_path = root / m.manifest
        if not manifest_path.exists():
            hard(f"{worker}/{m.manifest} not found")
        else:
            try:
                pr_ver = _lib.read_version(manifest_path)
            except (ValueError, FileNotFoundError) as e:
                hard(f"could not read version from {worker}/{m.manifest}: {e}")
                pr_ver = None
            if pr_ver is not None:
                try:
                    base_blob = subprocess.check_output(
                        ["git", "show", f"{args.base_ref}:{worker}/{m.manifest}"],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    )
                except subprocess.CalledProcessError:
                    base_blob = None
                # Only enforce when base resolves to a commit distinct from
                # HEAD. With a single-commit repo (e.g. brand-new branch on
                # this PR), base == HEAD and comparing to base is meaningless.
                try:
                    base_sha = subprocess.check_output(
                        ["git", "rev-parse", args.base_ref],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                    head_sha = subprocess.check_output(
                        ["git", "rev-parse", "HEAD"],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                except subprocess.CalledProcessError:
                    base_sha = head_sha = ""
                if base_blob is not None and base_sha != head_sha:
                    with tempfile.TemporaryDirectory() as td:
                        tmp = pathlib.Path(td) / m.manifest
                        tmp.write_text(base_blob)
                        try:
                            base_ver = _lib.read_version(tmp)
                        except (ValueError, FileNotFoundError):
                            base_ver = None
                    if base_ver is not None and _lib.parse_semver(pr_ver) < _lib.parse_semver(base_ver):
                        soft(
                            f"{worker}/{m.manifest} version {pr_ver} is less "
                            f"than base {base_ver}"
                        )

    # 4. tests/ exists and is non-empty
    tests_dir = root / "tests"
    if not tests_dir.exists():
        soft(f"{worker}/tests/ is missing")
    elif not any(tests_dir.iterdir()):
        soft(f"{worker}/tests/ is empty")

    # 5. Bundled workers must ship skills/SKILL.md within the size cap.
    if worker in BOOTSTRAP_WORKERS:
        skill_md = root / "skills" / "SKILL.md"
        legacy_skill_md = root / "skill.md"
        if not skill_md.exists() and legacy_skill_md.exists():
            skill_md = legacy_skill_md
        if not skill_md.exists():
            hard(
                f"{worker}/skills/SKILL.md is missing — bundled workers must ship one "
                f"(see docs/sops/binary-worker.md)"
            )
        elif skill_md.stat().st_size == 0:
            hard(
                f"{worker}/{skill_md.relative_to(root).as_posix()} is empty — "
                f"must contain the H1 + summary (see docs/sops/binary-worker.md)"
            )
        elif skill_md.stat().st_size > SKILL_MD_SIZE_CAP:
            hard(
                f"{worker}/{skill_md.relative_to(root).as_posix()} exceeds 256 KiB cap "
                f"({skill_md.stat().st_size} bytes; see docs/sops/binary-worker.md)"
            )

    for e in errs:
        print(f"::error::{e}")
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
