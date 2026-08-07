#!/usr/bin/env python3
"""Build the terminal, machine-readable result for a Release workflow run."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def parse_bool(value: str) -> bool:
    return value.strip().lower() == "true"


def build_result(args: argparse.Namespace) -> dict:
    jobs = {
        "setup": args.setup_result,
        "github_release": args.github_release_result,
        "binary_build": args.binary_build_result,
        "container_build": args.container_build_result,
        "bundle_build": args.bundle_build_result,
        "registry_publish": args.publish_result,
        "container_alias": args.container_alias_result,
        "candidate_ready": args.candidate_result,
    }
    dry_run = parse_bool(args.dry_run)
    staged = parse_bool(args.staged)
    interface_smoke = parse_bool(args.interface_smoke)

    required = ["setup"]
    if not dry_run:
        required.append("github_release")
    build_key = {
        "binary": "binary_build",
        "image": "container_build",
        "bundle": "bundle_build",
    }.get(args.deploy)
    if build_key:
        required.append(build_key)
    if not dry_run and interface_smoke:
        required.append("registry_publish")
        if args.deploy == "image":
            required.append("container_alias")
        if staged:
            required.append("candidate_ready")

    failed = [name for name in required if jobs.get(name) != "success"]
    irreversible = jobs["github_release"] == "success" or jobs["registry_publish"] == "success"
    status = "succeeded" if not failed else ("partial" if irreversible else "failed")

    phase = "preflight"
    if jobs["github_release"] == "success":
        phase = "github_release"
    if build_key and jobs.get(build_key) == "success":
        phase = "assets"
    if jobs["registry_publish"] == "success":
        phase = "registry"
    if args.deploy != "image" or jobs["container_alias"] == "success":
        if phase == "registry":
            phase = "channel_aligned"
    if staged and jobs["candidate_ready"] == "success":
        phase = "candidate_ready"
    if status == "succeeded":
        phase = "complete"

    return {
        "schema_version": 2,
        "repository": args.repository,
        "release_run_id": args.release_run_id,
        "run_attempt": args.run_attempt,
        "operation_id": args.operation_id,
        "step_id": args.step_id,
        "worker": args.worker,
        "version": args.version,
        "maturity": args.maturity,
        "release_tag": args.release_tag,
        "tag_sha": args.tag_sha,
        "source_sha": args.source_sha,
        "deploy": args.deploy,
        "registry_tag": args.registry_tag,
        "image_digest": args.image_digest or None,
        "dry_run": dry_run,
        "staged": staged,
        "status": status,
        "phase": phase,
        "failed_requirements": failed,
        "jobs": jobs,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    for name in (
        "repository",
        "release_run_id",
        "operation_id",
        "step_id",
        "worker",
        "version",
        "maturity",
        "release_tag",
        "tag_sha",
        "source_sha",
        "deploy",
        "registry_tag",
        "dry_run",
        "staged",
        "interface_smoke",
        "setup_result",
        "github_release_result",
        "binary_build_result",
        "container_build_result",
        "bundle_build_result",
        "publish_result",
        "container_alias_result",
        "candidate_result",
    ):
        parser.add_argument(f"--{name.replace('_', '-')}", required=True)
    # Compatibility with a rerun of a pre-centralization Release workflow.
    # The value is intentionally ignored: notification delivery is app-owned.
    parser.add_argument("--notification-result")
    parser.add_argument("--run-attempt", type=int, required=True)
    parser.add_argument("--image-digest", default="")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = build_result(args)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
