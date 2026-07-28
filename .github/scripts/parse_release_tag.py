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

# The registry stores `registry-tag` verbatim (a free-form string column), so a
# typo in the annotated tag message would silently create a dead channel that
# nothing resolves. Keep the accepted set closed here, matching the Create Tag
# workflow options.
RELEASE_CHANNELS = ("latest", "next", "rc", "beta", "alpha")


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

    if wm.deploy not in ("binary", "image", "bundle"):
        print(
            f"::error::{worker}: deploy must be binary|image|bundle (got {wm.deploy!r})",
            file=sys.stderr,
        )
        return 1

    registry_tag = _lib.read_tag_annotation(raw).get("registry-tag", "latest") or "latest"
    if registry_tag not in RELEASE_CHANNELS:
        print(
            f"::error::Unknown registry-tag {registry_tag!r} in the annotated tag "
            f"message; expected one of {', '.join(RELEASE_CHANNELS)}",
            file=sys.stderr,
        )
        return 1

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
