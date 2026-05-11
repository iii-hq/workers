#!/usr/bin/env python3
"""Parse a release tag and emit setup outputs for release.yml.

Writes 10 keys to $GITHUB_OUTPUT:
    tag, worker, version, deploy, language,
    bin, manifest, registry_tag, is_prerelease, dry_run
"""
from __future__ import annotations

import argparse
import os
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402


TAG_RE = re.compile(r"^([a-z0-9][a-z0-9_-]*)/v(.+)$")
DRY_RUN_RE = re.compile(r"-dry-run\.\d+$")
PRERELEASE_RE = re.compile(r"-[a-z]+\.\d+$")


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
    elif PRERELEASE_RE.search(version):
        dry_run, is_pre = "false", "true"
    else:
        dry_run, is_pre = "false", "false"

    worker_dir = pathlib.Path(worker)
    try:
        wm = _lib.read_iii_worker_yaml(worker_dir)
    except FileNotFoundError as e:
        print(f"::error::{e}", file=sys.stderr)
        return 1

    if wm.deploy not in ("binary", "image"):
        print(
            f"::error::{worker}: deploy must be binary|image (got {wm.deploy!r})",
            file=sys.stderr,
        )
        return 1

    registry_tag = _lib.read_tag_annotation(raw).get("registry-tag", "latest") or "latest"

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
    ]

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a") as f:
            for k, v in pairs:
                f.write(f"{k}={v}\n")

    print(
        f"::notice::release {worker} v{version} deploy={wm.deploy} "
        f"registry-tag={registry_tag}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
