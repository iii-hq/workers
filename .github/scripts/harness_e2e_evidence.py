#!/usr/bin/env python3
"""Build and validate deployed Harness E2E evidence."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

STABLE_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def build_evidence(args: argparse.Namespace) -> dict:
    ready = (
        args.suite_result == "success"
        and args.registry_tag == "next"
        and bool(args.release_run_id)
        and bool(args.release_tag)
    )
    return {
        "schema_version": 2,
        "repository": args.repository,
        "release_run_id": args.release_run_id,
        "e2e_run_id": args.e2e_run_id,
        "run_attempt": args.run_attempt,
        "operation_id": args.operation_id,
        "step_id": args.step_id,
        "release_tag": args.release_tag,
        "release_worker": args.release_worker,
        "release_version": args.release_version,
        "registry_tag": args.registry_tag,
        "cli_channel": args.cli_channel,
        "suite_result": args.suite_result,
        "e2e_ready": ready,
        "artifacts": ["harness-e2e-benchmark", "harness-e2e-dashboard"],
    }


def validate_evidence(args: argparse.Namespace) -> dict:
    evidence = json.loads(args.evidence.read_text())
    expected = {
        "schema_version": 2,
        "repository": args.repository,
        "release_run_id": args.release_run_id,
        "e2e_run_id": args.e2e_run_id,
        "release_tag": f"{args.worker}/v{args.version}",
        "release_worker": args.worker,
        "release_version": args.version,
        "registry_tag": "next",
        "suite_result": "success",
        "e2e_ready": True,
    }
    failures = [
        f"{key}: expected {value!r}, got {evidence.get(key)!r}"
        for key, value in expected.items()
        if evidence.get(key) != value
    ]
    if args.operation_id and evidence.get("operation_id") != args.operation_id:
        failures.append(
            f"operation_id: expected {args.operation_id!r}, got {evidence.get('operation_id')!r}"
        )
    if not STABLE_VERSION_RE.fullmatch(args.version):
        failures.append("version must be stable semver MAJOR.MINOR.PATCH")
    if not isinstance(evidence.get("run_attempt"), int) or evidence["run_attempt"] < 1:
        failures.append("run_attempt must be a positive integer")
    if failures:
        raise SystemExit("invalid Harness E2E evidence:\n- " + "\n- ".join(failures))
    return evidence


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    build = subparsers.add_parser("build")
    for name in (
        "repository",
        "release_run_id",
        "e2e_run_id",
        "operation_id",
        "step_id",
        "release_tag",
        "release_worker",
        "release_version",
        "registry_tag",
        "cli_channel",
        "suite_result",
    ):
        build.add_argument(f"--{name.replace('_', '-')}", required=True)
    build.add_argument("--run-attempt", type=int, required=True)
    build.add_argument("--output", type=Path, required=True)

    validate = subparsers.add_parser("validate")
    validate.add_argument("--evidence", type=Path, required=True)
    validate.add_argument("--repository", required=True)
    validate.add_argument("--release-run-id", required=True)
    validate.add_argument("--e2e-run-id", required=True)
    validate.add_argument("--worker", required=True)
    validate.add_argument("--version", required=True)
    validate.add_argument("--operation-id", default="")
    validate.add_argument("--output", type=Path)
    args = parser.parse_args()

    if args.command == "build":
        evidence = build_evidence(args)
        args.output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    else:
        evidence = validate_evidence(args)
        rendered = json.dumps(evidence, sort_keys=True)
        if args.output:
            args.output.write_text(rendered + "\n")
        print(rendered)


if __name__ == "__main__":
    main()
