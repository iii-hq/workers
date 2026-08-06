#!/usr/bin/env python3
"""Build and validate deployed Harness E2E evidence."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

from harness_e2e_profiles import CATALOG_PATH, load_profile_catalog

STABLE_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^[0-9a-f]{64}$")


def _json_object(value: str, field: str) -> dict[str, str]:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as error:
        raise SystemExit(f"invalid {field}: {error}") from error
    if not isinstance(parsed, dict):
        raise SystemExit(f"{field} must be a JSON object")
    return parsed


def _json_list(value: str, field: str, *, allow_empty: bool = False) -> list[str]:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as error:
        raise SystemExit(f"invalid {field}: {error}") from error
    if not isinstance(parsed, list) or (not parsed and not allow_empty):
        requirement = "a JSON array" if allow_empty else "a non-empty JSON array"
        raise SystemExit(f"{field} must be {requirement}")
    result: list[str] = []
    for item in parsed:
        if not isinstance(item, str) or not item:
            raise SystemExit(f"{field} entries must be non-empty strings")
        if item in result:
            raise SystemExit(f"{field} repeats {item}")
        result.append(item)
    return result


def _snapshot_coverage(
    path: Path | None, selected: list[str], requested_runs: int
) -> tuple[list[dict[str, str]], list[str], bool]:
    if path is None or not path.is_file():
        return [], [], False
    snapshot = json.loads(path.read_text())
    subjects = snapshot.get("subjects")
    if not isinstance(subjects, list) or not subjects:
        return [], [], False
    subject_entries: list[dict[str, str]] = []
    completed_sets: list[set[str]] = []
    structurally_complete = snapshot.get("requested_runs") == requested_runs
    for subject in subjects:
        if not isinstance(subject, dict):
            structurally_complete = False
            continue
        subject_id = subject.get("id")
        model = subject.get("model")
        provider = subject.get("provider")
        if not all(isinstance(value, str) and value for value in (subject_id, model, provider)):
            structurally_complete = False
            continue
        subject_entries.append({"id": subject_id, "model": model, "provider": provider})
        scenarios = subject.get("scenarios")
        if not isinstance(scenarios, list):
            structurally_complete = False
            completed_sets.append(set())
            continue
        completed = {
            entry.get("id")
            for entry in scenarios
            if isinstance(entry, dict)
            and isinstance(entry.get("id"), str)
            and entry.get("status") != "missing_report"
            and isinstance(entry.get("runs"), int)
            and entry.get("runs") == requested_runs
        }
        completed_sets.append(completed)
        structurally_complete = structurally_complete and completed == set(selected)
    completed = [
        scenario
        for scenario in selected
        if completed_sets and all(scenario in values for values in completed_sets)
    ]
    return subject_entries, completed, structurally_complete


def build_evidence(args: argparse.Namespace) -> dict[str, Any]:
    stack_versions = _json_object(args.stack_versions, "stack_versions")
    if not stack_versions:
        stack_versions = {args.release_worker: args.release_version}
    if stack_versions.get(args.release_worker) != args.release_version:
        raise SystemExit("stack_versions must contain the exact release worker version")
    if args.validation_profile not in {"release", "custom", "full"}:
        raise SystemExit("validation_profile must be release|custom|full")
    catalog = load_profile_catalog()
    selected = _json_list(args.scenarios_json, "scenarios_json", allow_empty=True)
    required = _json_list(
        args.required_scenarios_json, "required_scenarios_json", allow_empty=True
    )
    if not selected and args.validation_profile == "release":
        selected = list(catalog.release_scenarios)
    elif not selected and args.validation_profile == "full":
        selected = list(catalog.ids)
    if not required:
        required = list(catalog.release_scenarios)
    profile_digest = args.profile_digest or catalog.profile_digest
    if not DIGEST_RE.fullmatch(profile_digest):
        raise SystemExit("profile_digest must be a SHA-256 digest")
    if not SHA_RE.fullmatch(args.catalog_sha) or not SHA_RE.fullmatch(args.suite_sha):
        raise SystemExit("catalog_sha and suite_sha must be full commit SHAs")
    subjects, completed, coverage_complete = _snapshot_coverage(
        args.benchmark_snapshot, selected, args.runs
    )
    ready = (
        args.suite_result == "success"
        and args.registry_tag == "next"
        and bool(args.release_run_id)
        and bool(args.release_tag)
        and bool(stack_versions)
        and bool(selected)
        and bool(required)
        and coverage_complete
    )
    promotion_ready = ready and set(required).issubset(completed)
    return {
        "schema_version": 3,
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
        "suite_sha": args.suite_sha,
        "catalog_sha": args.catalog_sha,
        "validation_profile": args.validation_profile,
        "profile_digest": profile_digest,
        "selected_scenarios": selected,
        "completed_scenarios": completed,
        "required_scenarios": required,
        "subjects": subjects,
        "runs": args.runs,
        "stack_versions": stack_versions,
        "e2e_ready": ready,
        "promotion_ready": promotion_ready,
        "artifacts": ["harness-e2e-benchmark", "harness-e2e-dashboard"],
    }


def _legacy_failures(evidence: dict[str, Any], args: argparse.Namespace) -> list[str]:
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
    return [
        f"{key}: expected {value!r}, got {evidence.get(key)!r}"
        for key, value in expected.items()
        if evidence.get(key) != value
    ]


def validate_evidence(args: argparse.Namespace) -> dict[str, Any]:
    evidence = json.loads(args.evidence.read_text())
    schema_version = evidence.get("schema_version")
    if schema_version == 2:
        failures = _legacy_failures(evidence, args)
    elif schema_version == 3:
        catalog = load_profile_catalog(args.catalog)
        expected = {
            "schema_version": 3,
            "repository": args.repository,
            "release_run_id": args.release_run_id,
            "e2e_run_id": args.e2e_run_id,
            "release_tag": f"{args.worker}/v{args.version}",
            "release_worker": args.worker,
            "release_version": args.version,
            "registry_tag": "next",
            "suite_result": "success",
            "e2e_ready": True,
            "promotion_ready": True,
            "profile_digest": catalog.profile_digest,
            "required_scenarios": list(catalog.release_scenarios),
        }
        failures = [
            f"{key}: expected {value!r}, got {evidence.get(key)!r}"
            for key, value in expected.items()
            if evidence.get(key) != value
        ]
        selected = evidence.get("selected_scenarios")
        completed = evidence.get("completed_scenarios")
        if not isinstance(selected, list) or not set(catalog.release_scenarios).issubset(selected):
            failures.append("selected_scenarios do not cover the current release gate")
        if completed != selected:
            failures.append("completed_scenarios must exactly match selected_scenarios")
        if not SHA_RE.fullmatch(str(evidence.get("catalog_sha", ""))):
            failures.append("catalog_sha must be a full commit SHA")
        if not SHA_RE.fullmatch(str(evidence.get("suite_sha", ""))):
            failures.append("suite_sha must be a full commit SHA")
    else:
        failures = [f"schema_version: expected 2 or 3, got {schema_version!r}"]

    stack_versions = evidence.get("stack_versions")
    if not isinstance(stack_versions, dict) or stack_versions.get(args.worker) != args.version:
        failures.append("stack_versions must contain the exact released worker version")
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
        "stack_versions",
        "validation_profile",
        "scenarios_json",
        "required_scenarios_json",
        "profile_digest",
        "catalog_sha",
        "suite_sha",
    ):
        build.add_argument(f"--{name.replace('_', '-')}", required=True)
    build.add_argument("--runs", type=int, required=True)
    build.add_argument("--benchmark-snapshot", type=Path)
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
    validate.add_argument("--catalog", type=Path, default=CATALOG_PATH)
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
