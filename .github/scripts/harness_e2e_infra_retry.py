#!/usr/bin/env python3
"""Decide whether a deployed Harness E2E provisioning attempt may be retried."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


RETRYABLE_PHASES = {"bootstrap", "registry"}
INTERRUPTED_EXIT_CODES = {124, 130, 137, 143}


def load_deployment(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def retry_eligible(
    *,
    deployment: dict[str, Any] | None,
    exit_code: int,
    results_exist: bool,
) -> bool:
    if exit_code == 0 or exit_code in INTERRUPTED_EXIT_CODES or results_exist:
        return False
    return bool(
        deployment
        and deployment.get("status") == "infra_failed"
        and deployment.get("failure_phase") in RETRYABLE_PHASES
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--deployment", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--exit-code", type=int, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    eligible = retry_eligible(
        deployment=load_deployment(args.deployment),
        exit_code=args.exit_code,
        results_exist=args.results.is_file(),
    )
    print(json.dumps({"retry_eligible": eligible}))
    return 0 if eligible else 1


if __name__ == "__main__":
    raise SystemExit(main())
