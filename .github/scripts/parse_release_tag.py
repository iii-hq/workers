#!/usr/bin/env python3
"""Parse a release tag and emit setup outputs for release.yml.

Writes 13 keys to $GITHUB_OUTPUT:
    tag, worker, version, deploy, language, bin, manifest,
    registry_tag, is_prerelease, dry_run, targets, experimental, tag_sha
"""
from __future__ import annotations

import argparse
import os
import pathlib
import re
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402


TAG_RE = re.compile(r"^([a-z0-9][a-z0-9_-]*)/v(.+)$")
DRY_RUN_RE = re.compile(r"-dry-run\.\d+$")
STABLE_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("raw_tag")
    args = p.parse_args(argv)

    raw = args.raw_tag.strip()
    m = TAG_RE.match(raw)
    if not m:
        print(f"::error::Invalid tag shape: {raw}", file=sys.stderr)
        return 1
    worker, version = m.group(1), m.group(2)

    if DRY_RUN_RE.search(version):
        dry_run, is_pre = "true", "true"
    elif STABLE_VERSION_RE.fullmatch(version):
        dry_run, is_pre = "false", "false"
    else:
        dry_run, is_pre = "false", "true"

    worker_dir = pathlib.Path(worker)
    try:
        wm = _lib.read_iii_worker_yaml(worker_dir)
    except FileNotFoundError as e:
        print(f"::error::{e}", file=sys.stderr)
        return 1

    if wm.deploy not in ("binary", "image", "bundle"):
        print(
            f"::error::{worker}: deploy must be binary|image|bundle (got {wm.deploy!r})",
            file=sys.stderr,
        )
        return 1

    annotation = _lib.read_tag_annotation(raw)
    registry_tag = annotation.get("registry-tag", "latest") or "latest"
    tag_sha = subprocess.check_output(
        ["git", "rev-list", "-n", "1", raw], text=True
    ).strip()

    # Anything but a literal `true` is false: a lightweight tag, a missing
    # line, or a typo publishes as stable. Marking a worker experimental is
    # the deliberate choice, so it takes the exact word.
    experimental = "true" if annotation.get("experimental", "").strip().lower() == "true" else "false"

    # `targets` is an optional iii.worker.yaml field that can be either a
    # list (`- aarch64-apple-darwin\n- x86_64-unknown-linux-gnu`) or a comma
    # string. Normalise to a single comma-joined string for the workflow.
    targets_raw = wm.raw.get("targets")
    if isinstance(targets_raw, list):
        targets = ",".join(str(t).strip() for t in targets_raw if str(t).strip())
    elif isinstance(targets_raw, str):
        targets = targets_raw.strip()
    else:
        targets = ""

    pairs = [
        ("tag", raw),
        ("worker", worker),
        ("version", version),
        ("deploy", wm.deploy),
        ("language", wm.language or ""),
        ("bin", wm.bin or wm.name),
        ("manifest", wm.manifest or ""),
        ("registry_tag", registry_tag),
        ("is_prerelease", is_pre),
        ("dry_run", dry_run),
        ("targets", targets),
        ("experimental", experimental),
        ("tag_sha", tag_sha),
    ]

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a") as f:
            for k, v in pairs:
                f.write(f"{k}={v}\n")

    print(
        f"::notice::release {worker} v{version} deploy={wm.deploy} "
        f"registry-tag={registry_tag} experimental={experimental}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
