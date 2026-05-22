#!/usr/bin/env python3
"""Per-worker pr-checks validation.

Enforces:
    1. README.md exists and is non-empty.
    2. iii.worker.yaml parses and has required fields + valid enum values.
    3. The manifest version on this ref is greater than or equal to on --base-ref.
    4. tests/ exists and is non-empty.
    5. For workers in BOOTSTRAP_WORKERS, skill.md exists, is non-empty, and is
       within the 256 KiB cap — the harness bootstraps these onto disk via
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


# Workers the harness materialises into ./data/skills/<name>/index.md on
# boot via skills::download. Must match harness/src/skills.rs::BOOTSTRAP_NAMES.
BOOTSTRAP_WORKERS = frozenset({
    "iii-directory",
    "shell",
})

SKILL_MD_SIZE_CAP = 256 * 1024  # 256 KiB


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

    # 5. Bundled workers must ship skill.md within the size cap.
    if worker in BOOTSTRAP_WORKERS:
        skill_md = root / "skill.md"
        if not skill_md.exists():
            hard(
                f"{worker}/skill.md is missing — bundled workers must ship one "
                f"(see binary-worker.md)"
            )
        elif skill_md.stat().st_size == 0:
            hard(
                f"{worker}/skill.md is empty — must contain the H1 + summary "
                f"(see binary-worker.md)"
            )
        elif skill_md.stat().st_size > SKILL_MD_SIZE_CAP:
            hard(
                f"{worker}/skill.md exceeds 256 KiB cap "
                f"({skill_md.stat().st_size} bytes; see binary-worker.md)"
            )

    for e in errs:
        print(f"::error::{e}")
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
