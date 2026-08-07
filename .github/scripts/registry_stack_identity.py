#!/usr/bin/env python3
"""Create a deterministic identity for a Registry-resolved iii.lock."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any

import yaml


WORKER_NAME = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
EXACT_VERSION = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(-(experimental|alpha|beta))?$"
)


def stack_identity(lock_path: Path) -> dict[str, Any]:
    try:
        document = yaml.safe_load(lock_path.read_text()) or {}
    except (OSError, yaml.YAMLError) as error:
        raise SystemExit(f"invalid_lock: cannot read {lock_path}: {error}") from error

    workers = document.get("workers") if isinstance(document, dict) else None
    if not isinstance(workers, dict) or not workers:
        raise SystemExit("invalid_lock: workers must be a non-empty mapping")

    versions: dict[str, str] = {}
    for worker, record in sorted(workers.items()):
        if not isinstance(worker, str) or not WORKER_NAME.fullmatch(worker):
            raise SystemExit(f"invalid_lock: invalid worker name {worker!r}")
        if not isinstance(record, dict):
            raise SystemExit(f"invalid_lock: {worker} must be a mapping")
        version = record.get("version")
        if not isinstance(version, str) or not EXACT_VERSION.fullmatch(version):
            raise SystemExit(
                f"invalid_lock: {worker} must have an exact published version"
            )
        versions[worker] = version

    if "harness" not in versions:
        raise SystemExit("invalid_lock: harness is required in the resolved stack")

    return {
        "schema_version": 1,
        "lock_digest": hashlib.sha256(lock_path.read_bytes()).hexdigest(),
        "stack_versions": versions,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    identity = stack_identity(args.lock)
    rendered = json.dumps(identity, sort_keys=True)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n")
    print(rendered)


if __name__ == "__main__":
    main()
