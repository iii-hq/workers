#!/usr/bin/env python3
"""Build and validate evidence for a staged worker release."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def parse_bool(value: str) -> bool:
    normalized = value.strip().lower()
    if normalized == "true":
        return True
    if normalized == "false":
        return False
    raise argparse.ArgumentTypeError("expected true or false")


def build_evidence(args: argparse.Namespace) -> dict:
    results = {
        "publish": args.publish_result,
        "candidate_smoke": args.candidate_smoke_result,
        "harness_quickstart": args.harness_quickstart_result,
        "harness_e2e": args.harness_e2e_result,
    }
    candidate_ready = results["publish"] == "success" and results["candidate_smoke"] == "success"
    if args.harness_gate_required:
        candidate_ready = (
            candidate_ready
            and results["harness_quickstart"] == "success"
            and results["harness_e2e"] == "success"
        )

    return {
        "schema_version": 1,
        "repository": args.repository,
        "release_run_id": args.release_run_id,
        "run_attempt": args.run_attempt,
        "tag_sha": args.tag_sha,
        "release_tag": args.release_tag,
        "worker": args.worker,
        "version": args.version,
        "deploy": args.deploy,
        "registry_tag": args.registry_tag,
        "harness_gate_required": args.harness_gate_required,
        "promotable": args.promotable,
        "candidate_ready": candidate_ready,
        "results": results,
    }


def validate_evidence(args: argparse.Namespace) -> dict:
    evidence = json.loads(args.evidence.read_text())
    failures: list[str] = []

    expected = {
        "schema_version": 1,
        "repository": args.repository,
        "release_run_id": args.release_run_id,
        "release_tag": f"{args.worker}/v{args.version}",
        "worker": args.worker,
        "version": args.version,
        "registry_tag": "next",
        "candidate_ready": True,
        "promotable": True,
    }
    for key, value in expected.items():
        if evidence.get(key) != value:
            failures.append(f"{key}: expected {value!r}, got {evidence.get(key)!r}")

    if not VERSION_RE.fullmatch(args.version):
        failures.append("version must be stable semver MAJOR.MINOR.PATCH")
    if not SHA_RE.fullmatch(str(evidence.get("tag_sha", ""))):
        failures.append("tag_sha must be a full lowercase commit SHA")
    if not isinstance(evidence.get("run_attempt"), int) or evidence["run_attempt"] < 1:
        failures.append("run_attempt must be a positive integer")

    results = evidence.get("results")
    if not isinstance(results, dict):
        failures.append("results must be an object")
    else:
        if results.get("publish") != "success":
            failures.append("publish gate did not succeed")
        if results.get("candidate_smoke") != "success":
            failures.append("candidate smoke gate did not succeed")
        if evidence.get("harness_gate_required"):
            if results.get("harness_quickstart") != "success":
                failures.append("Harness quickstart gate did not succeed")
            if results.get("harness_e2e") != "success":
                failures.append("Harness E2E gate did not succeed")

    if failures:
        raise SystemExit("invalid release candidate evidence:\n- " + "\n- ".join(failures))
    return evidence


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    build = subparsers.add_parser("build")
    build.add_argument("--repository", required=True)
    build.add_argument("--release-run-id", required=True)
    build.add_argument("--run-attempt", type=int, required=True)
    build.add_argument("--tag-sha", required=True)
    build.add_argument("--release-tag", required=True)
    build.add_argument("--worker", required=True)
    build.add_argument("--version", required=True)
    build.add_argument("--deploy", choices=("binary", "image", "bundle"), required=True)
    build.add_argument("--registry-tag", required=True)
    build.add_argument("--harness-gate-required", type=parse_bool, required=True)
    build.add_argument("--promotable", type=parse_bool, required=True)
    build.add_argument("--publish-result", required=True)
    build.add_argument("--candidate-smoke-result", required=True)
    build.add_argument("--harness-quickstart-result", required=True)
    build.add_argument("--harness-e2e-result", required=True)
    build.add_argument("--output", type=Path, required=True)

    validate = subparsers.add_parser("validate")
    validate.add_argument("--evidence", type=Path, required=True)
    validate.add_argument("--repository", required=True)
    validate.add_argument("--release-run-id", required=True)
    validate.add_argument("--worker", required=True)
    validate.add_argument("--version", required=True)
    validate.add_argument("--output", type=Path)
    return parser


def main() -> None:
    args = build_parser().parse_args()
    if args.command == "build":
        evidence = build_evidence(args)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    else:
        evidence = validate_evidence(args)
        rendered = json.dumps(evidence, sort_keys=True)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered + "\n")
        print(rendered)


if __name__ == "__main__":
    main()
