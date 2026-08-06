#!/usr/bin/env python3
"""Verify that iii.lock resolved the released worker and required stack."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import yaml


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--required", action="append", default=[])
    parser.add_argument("--expected-versions-json", default="{}")
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def load_yaml(path: Path) -> dict:
    value = yaml.safe_load(path.read_text()) or {}
    if not isinstance(value, dict):
        raise SystemExit(f"invalid_yaml: {path} must contain a mapping")
    return value


def main() -> None:
    args = parse_args()
    lock = load_yaml(args.lock)
    workers = lock.get("workers") or {}
    if not isinstance(workers, dict):
        raise SystemExit("invalid_lock: workers must be a mapping")

    record = workers.get(args.worker)
    actual_version = record.get("version") if isinstance(record, dict) else None
    if str(actual_version or "") != args.version:
        raise SystemExit(
            f"artifact_mismatch: expected {args.worker} {args.version}, "
            f"resolved {actual_version or 'unknown'}"
        )

    required = set(args.required)
    if args.manifest:
        manifest = load_yaml(args.manifest)
        dependencies = manifest.get("dependencies") or {}
        if not isinstance(dependencies, dict):
            raise SystemExit("invalid_manifest: dependencies must be a mapping")
        required.update(dependencies)

    missing = sorted(required - workers.keys())
    if missing:
        raise SystemExit(
            "stack_incomplete: required workers absent from iii.lock: "
            + ", ".join(missing)
        )

    try:
        expected_versions = json.loads(args.expected_versions_json)
    except json.JSONDecodeError as error:
        raise SystemExit(f"invalid_expected_versions: {error}") from error
    if not isinstance(expected_versions, dict):
        raise SystemExit("invalid_expected_versions: expected a JSON object")

    mismatches: list[str] = []
    resolved_versions: dict[str, str] = {}
    for worker, expected in sorted(expected_versions.items()):
        record = workers.get(worker)
        resolved = record.get("version") if isinstance(record, dict) else None
        resolved_versions[str(worker)] = str(resolved or "")
        if str(resolved or "") != str(expected):
            mismatches.append(f"{worker}: expected {expected}, resolved {resolved or 'unknown'}")
    if mismatches:
        raise SystemExit("stack_version_mismatch: " + "; ".join(mismatches))

    result = {
        "worker": args.worker,
        "expected_version": args.version,
        "actual_version": str(actual_version),
        "required_workers": sorted(required),
        "expected_versions": expected_versions,
        "resolved_versions": resolved_versions,
    }
    rendered = json.dumps(result, sort_keys=True)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n")
    print(rendered)


if __name__ == "__main__":
    main()
