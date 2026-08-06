#!/usr/bin/env python3
"""Parse a release tag and emit setup outputs for release.yml.

Writes release identity and manifest metadata to $GITHUB_OUTPUT:
    tag, worker, version, deploy, language, bin, manifest,
    registry_tag, is_prerelease, dry_run, targets, experimental, tag_sha,
    release_contract, operation_id, step_id, source_sha, maturity
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
from release_catalog import load_catalog  # noqa: E402


TAG_RE = re.compile(r"^([a-z0-9][a-z0-9_-]*)/v(.+)$")
DRY_RUN_RE = re.compile(r"-dry-run\.\d+$")
STABLE_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("raw_tag")
    p.add_argument(
        "--source-dir",
        type=pathlib.Path,
        default=pathlib.Path("."),
        help="checkout containing the worker at the release tag",
    )
    args = p.parse_args(argv)

    raw = args.raw_tag.strip()
    m = TAG_RE.match(raw)
    if not m:
        print(f"::error::Invalid tag shape: {raw}", file=sys.stderr)
        return 1
    worker, version = m.group(1), m.group(2)

    annotation = _lib.read_tag_annotation(raw)
    release_contract = annotation.get("release-contract", "1").strip() or "1"
    if release_contract not in {"1", "2"}:
        print(f"::error::Unsupported release-contract: {release_contract}", file=sys.stderr)
        return 1

    try:
        catalog = load_catalog()
    except (FileNotFoundError, ValueError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    worker_config = catalog.get(worker)
    if not worker_config or worker_config.get("release_workflow") != "release.yml":
        print(f"::error::{worker} is not a standard releasable worker", file=sys.stderr)
        return 1

    if DRY_RUN_RE.search(version) and release_contract == "1":
        dry_run, is_pre = "true", "true"
    else:
        try:
            maturity = _lib.release_maturity(version)
        except ValueError as error:
            if release_contract == "2":
                print(f"::error::{error}", file=sys.stderr)
                return 1
            maturity = "stable" if STABLE_VERSION_RE.fullmatch(version) else "legacy-prerelease"
        dry_run = "false"
        is_pre = "false" if maturity == "stable" else "true"

    if DRY_RUN_RE.search(version) and release_contract == "1":
        maturity = "legacy-dry-run"

    worker_dir = args.source_dir / worker
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
    if not wm.manifest:
        print(f"::error::{worker}: iii.worker.yaml has no manifest", file=sys.stderr)
        return 1
    try:
        manifest_version = _lib.read_version(worker_dir / wm.manifest)
    except (FileNotFoundError, ValueError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    if manifest_version != version:
        print(
            f"::error::tag version {version} does not match {worker}/{wm.manifest} ({manifest_version})",
            file=sys.stderr,
        )
        return 1

    registry_tag = annotation.get("registry-tag", "next").strip() or "next"
    if registry_tag not in {"next", "latest"}:
        print(f"::error::registry-tag must be next|latest (got {registry_tag!r})", file=sys.stderr)
        return 1
    if is_pre == "true" and dry_run != "true" and registry_tag != "next":
        print(f"::error::prerelease {version} must publish to next", file=sys.stderr)
        return 1
    if registry_tag == "latest" and not worker_config["allow_direct_latest"]:
        print(f"::error::{worker} cannot publish directly to latest", file=sys.stderr)
        return 1

    operation_id = annotation.get("operation-id", "legacy") or "legacy"
    step_id = annotation.get("step-id", "legacy") or "legacy"
    source_sha = annotation.get("source-sha", "unknown") or "unknown"
    if release_contract == "2":
        expected = {
            "worker": worker,
            "version": version,
            "maturity": maturity,
        }
        for key, value in expected.items():
            if annotation.get(key) != value:
                print(
                    f"::error::tag annotation {key} must be {value!r} (got {annotation.get(key)!r})",
                    file=sys.stderr,
                )
                return 1
        if not operation_id.strip() or not step_id.strip():
            print("::error::v2 tags require operation-id and step-id", file=sys.stderr)
            return 1
        if source_sha != "unknown" and not re.fullmatch(r"[0-9a-f]{40}", source_sha):
            print("::error::source-sha must be unknown or a full lowercase commit SHA", file=sys.stderr)
            return 1
    tag_sha = subprocess.check_output(
        ["git", "rev-list", "-n", "1", raw], text=True
    ).strip()

    # Anything but a literal `true` is false: a lightweight tag, a missing
    # line, or a typo publishes as stable. Marking a worker experimental is
    # the deliberate choice, so it takes the exact word.
    experimental_raw = annotation.get("experimental", "").strip().lower()
    if release_contract == "2" and experimental_raw not in {"true", "false"}:
        print("::error::v2 tags require experimental: true|false", file=sys.stderr)
        return 1
    experimental = "true" if experimental_raw == "true" else "false"

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
        ("release_contract", release_contract),
        ("operation_id", operation_id),
        ("step_id", step_id),
        ("source_sha", source_sha),
        ("maturity", maturity),
    ]

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a") as f:
            for k, v in pairs:
                f.write(f"{k}={v}\n")

    print(
        f"::notice::release {worker} v{version} deploy={wm.deploy} "
        f"maturity={maturity} registry-tag={registry_tag} experimental={experimental}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
