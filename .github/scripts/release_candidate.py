#!/usr/bin/env python3
"""Build and validate evidence for a staged worker release."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-(experimental|alpha|beta))?$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


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
        "container_alias": args.container_alias_result,
    }
    candidate_ready = results["publish"] == "success" and results["candidate_smoke"] == "success"
    if args.deploy == "image":
        candidate_ready = candidate_ready and results["container_alias"] == "success"

    return {
        "schema_version": 2,
        "repository": args.repository,
        "release_run_id": args.release_run_id,
        "evidence_run_id": args.evidence_run_id,
        "run_attempt": args.run_attempt,
        "tag_sha": args.tag_sha,
        "source_sha": args.source_sha,
        "release_tag": args.release_tag,
        "worker": args.worker,
        "version": args.version,
        "maturity": args.maturity,
        "deploy": args.deploy,
        "registry_tag": args.registry_tag,
        "operation_id": args.operation_id,
        "step_id": args.step_id,
        "image_digest": args.image_digest or None,
        "promotable": args.promotable,
        "candidate_ready": candidate_ready,
        "results": results,
    }


def validate_evidence(args: argparse.Namespace) -> dict:
    evidence = json.loads(args.evidence.read_text())
    failures: list[str] = []
    schema_version = evidence.get("schema_version")

    expected = {
        "schema_version": schema_version,
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

    if schema_version not in {1, 2}:
        failures.append(f"schema_version: expected 1|2, got {schema_version!r}")
    if schema_version == 1 and args.evidence_run_id not in {"", args.release_run_id}:
        failures.append("schema v1 evidence must come from the original Release run")

    if schema_version == 2 and args.evidence_run_id and evidence.get("evidence_run_id") != args.evidence_run_id:
        failures.append(
            f"evidence_run_id: expected {args.evidence_run_id!r}, got {evidence.get('evidence_run_id')!r}"
        )
    if schema_version == 2 and args.operation_id and evidence.get("operation_id") != args.operation_id:
        failures.append(
            f"operation_id: expected {args.operation_id!r}, got {evidence.get('operation_id')!r}"
        )

    version_match = VERSION_RE.fullmatch(args.version)
    if not version_match:
        failures.append("version must use MAJOR.MINOR.PATCH[-experimental|-alpha|-beta]")
    if not SHA_RE.fullmatch(str(evidence.get("tag_sha", ""))):
        failures.append("tag_sha must be a full lowercase commit SHA")
    if schema_version == 2:
        source_sha = str(evidence.get("source_sha", ""))
        if source_sha != "unknown" and not SHA_RE.fullmatch(source_sha):
            failures.append("source_sha must be unknown or a full lowercase commit SHA")
    if not isinstance(evidence.get("run_attempt"), int) or evidence["run_attempt"] < 1:
        failures.append("run_attempt must be a positive integer")
    if schema_version == 2 and version_match:
        expected_maturity = version_match.group(1) or "stable"
        if evidence.get("maturity") != expected_maturity:
            failures.append(
                f"maturity: expected {expected_maturity!r}, got {evidence.get('maturity')!r}"
            )
    if schema_version == 2 and not str(evidence.get("operation_id", "")).strip():
        failures.append("operation_id must be present")

    results = evidence.get("results")
    if not isinstance(results, dict):
        failures.append("results must be an object")
    else:
        if results.get("publish") != "success":
            failures.append("publish gate did not succeed")
        if results.get("candidate_smoke") != "success":
            failures.append("candidate smoke gate did not succeed")
        if schema_version == 1 and evidence.get("harness_gate_required"):
            if results.get("harness_quickstart") != "success":
                failures.append("Harness quickstart gate did not succeed")
            if results.get("harness_e2e") != "success":
                failures.append("Harness E2E gate did not succeed")
        if schema_version == 2 and evidence.get("deploy") == "image":
            if results.get("container_alias") != "success":
                failures.append("container alias gate did not succeed")
            if not DIGEST_RE.fullmatch(str(evidence.get("image_digest", ""))):
                failures.append("image_digest must be a sha256 digest for image releases")

    if failures:
        raise SystemExit("invalid release candidate evidence:\n- " + "\n- ".join(failures))
    return evidence


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    build = subparsers.add_parser("build")
    build.add_argument("--repository", required=True)
    build.add_argument("--release-run-id", required=True)
    build.add_argument("--evidence-run-id", required=True)
    build.add_argument("--run-attempt", type=int, required=True)
    build.add_argument("--tag-sha", required=True)
    build.add_argument("--source-sha", required=True)
    build.add_argument("--release-tag", required=True)
    build.add_argument("--worker", required=True)
    build.add_argument("--version", required=True)
    build.add_argument("--maturity", choices=("experimental", "alpha", "beta", "stable"), required=True)
    build.add_argument("--deploy", choices=("binary", "image", "bundle"), required=True)
    build.add_argument("--registry-tag", required=True)
    build.add_argument("--operation-id", required=True)
    build.add_argument("--step-id", required=True)
    build.add_argument("--image-digest", default="")
    build.add_argument("--promotable", type=parse_bool, required=True)
    build.add_argument("--publish-result", required=True)
    build.add_argument("--candidate-smoke-result", required=True)
    build.add_argument("--container-alias-result", required=True)
    build.add_argument("--output", type=Path, required=True)

    validate = subparsers.add_parser("validate")
    validate.add_argument("--evidence", type=Path, required=True)
    validate.add_argument("--repository", required=True)
    validate.add_argument("--release-run-id", required=True)
    validate.add_argument("--evidence-run-id", default="")
    validate.add_argument("--operation-id", default="")
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
