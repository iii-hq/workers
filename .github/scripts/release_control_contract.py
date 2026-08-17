#!/usr/bin/env python3
"""Strict Release Control dispatch and factual evidence contract."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path
from uuid import UUID


KINDS = {
    "create_tag",
    "release",
    "registry_promotion",
    "candidate_smoke",
    "container_alias",
    "github_release",
    "release_verify",
    "harness_validation",
    "test",
}


def _uuid(value: str, field: str) -> str:
    try:
        parsed = UUID(value)
    except ValueError as error:
        raise ValueError(f"{field} must be a UUID") from error
    if str(parsed) != value.lower():
        raise ValueError(f"{field} must use canonical UUID form")
    return str(parsed)


def validate_dispatch(args: argparse.Namespace) -> int:
    expected_bot = args.expected_bot.strip()
    if not expected_bot:
        raise ValueError("RELEASE_CONTROL_BOT_LOGIN is required")
    _uuid(args.operation_id, "operation_id")
    _uuid(args.step_id, "step_id")
    if args.actor != expected_bot or args.triggering_actor != expected_bot:
        raise ValueError(
            f"dispatch rejected: actor={args.actor!r}, triggering_actor={args.triggering_actor!r}, "
            f"expected={expected_bot!r}"
        )
    if args.run_attempt is None:
        raise ValueError("GITHUB_RUN_ATTEMPT or --run-attempt is required")
    if args.mutating and args.run_attempt != 1:
        raise ValueError("mutating Release Control executors cannot be rerun; create a recovery operation")
    return 0


def _json(value: str, field: str, expected: type) -> object:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as error:
        raise ValueError(f"{field} must be valid JSON") from error
    if not isinstance(parsed, expected):
        raise ValueError(f"{field} must be a JSON {expected.__name__}")
    return parsed


def write_result(args: argparse.Namespace) -> int:
    if args.kind not in KINDS:
        raise ValueError(f"unsupported execution kind: {args.kind}")
    operation_id = _uuid(args.operation_id, "operation_id")
    step_id = _uuid(args.step_id, "step_id")
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        raise ValueError("sha must be a full lowercase commit SHA")
    payload = {
        "schema_version": 1,
        "kind": args.kind,
        "repository": args.repository,
        "operation_id": operation_id,
        "step_id": step_id,
        "run": {
            "id": args.run_id,
            "attempt": args.run_attempt,
            "workflow": args.workflow,
            "event": args.event,
            "sha": args.sha,
        },
        "subject": _json(args.subject, "subject", dict),
        "checks": _json(args.checks, "checks", list),
        "effects": _json(args.effects, "effects", list),
        "outputs": _json(args.outputs, "outputs", dict),
    }
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    print(json.dumps(payload, sort_keys=True))
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)

    validate = commands.add_parser("validate-dispatch")
    validate.add_argument("--operation-id", required=True)
    validate.add_argument("--step-id", required=True)
    validate.add_argument("--actor", default=os.environ.get("GITHUB_ACTOR", ""))
    validate.add_argument("--triggering-actor", default=os.environ.get("GITHUB_TRIGGERING_ACTOR", ""))
    validate.add_argument("--expected-bot", default=os.environ.get("RELEASE_CONTROL_BOT_LOGIN", ""))
    validate.add_argument("--run-attempt", type=int, default=os.environ.get("GITHUB_RUN_ATTEMPT"))
    validate.add_argument("--mutating", action="store_true")
    validate.set_defaults(handler=validate_dispatch)

    write = commands.add_parser("write-result")
    write.add_argument("--kind", required=True)
    write.add_argument("--repository", required=True)
    write.add_argument("--operation-id", required=True)
    write.add_argument("--step-id", required=True)
    write.add_argument("--run-id", type=int, required=True)
    write.add_argument("--run-attempt", type=int, required=True)
    write.add_argument("--workflow", required=True)
    write.add_argument("--event", required=True)
    write.add_argument("--sha", required=True)
    write.add_argument("--subject", required=True)
    write.add_argument("--checks", required=True)
    write.add_argument("--effects", required=True)
    write.add_argument("--outputs", required=True)
    write.add_argument("--output", type=Path, required=True)
    write.set_defaults(handler=write_result)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        return args.handler(args)
    except ValueError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
