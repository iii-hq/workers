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
    "prepare_release",
    "publish_candidate",
    "publish_stable",
    "registry_finalize",
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


def _optional_uuid(value: str | None, field: str) -> str | None:
    if value in (None, ""):
        return None
    return _uuid(value, field)


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
    if args.plan_hash and not re.fullmatch(r"[0-9a-f]{64}", args.plan_hash):
        raise ValueError("plan_hash must be a 64-char lowercase SHA-256")
    if args.dispatch_nonce:
        _uuid(args.dispatch_nonce, "dispatch_nonce")
    for value, field in ((args.source_sha, "source_sha"), (args.prepared_sha, "prepared_sha")):
        if value and not re.fullmatch(r"[0-9a-f]{40}", value):
            raise ValueError(f"{field} must be a full lowercase commit SHA")
    candidate_pattern = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)-rc\.[1-9][0-9]*"
    if args.candidate_version and not re.fullmatch(candidate_pattern, args.candidate_version):
        raise ValueError("candidate_version must be x.y.z-rc.N")
    if args.stable_version and not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", args.stable_version):
        raise ValueError("stable_version must be x.y.z")
    if args.candidate_version and args.stable_version and args.candidate_version.split("-", 1)[0] != args.stable_version:
        raise ValueError("candidate_version and stable_version must share the same core")
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
    release_identity = {
        key: value
        for key, value in {
            "release_intent_id": _optional_uuid(args.release_intent_id, "release_intent_id"),
            "candidate_id": _optional_uuid(args.candidate_id, "candidate_id"),
            "attempt_id": _optional_uuid(args.attempt_id, "attempt_id"),
            "plan_hash": args.plan_hash or None,
            "dispatch_nonce": args.dispatch_nonce or None,
            "candidate_version": args.candidate_version or None,
            "stable_version": args.stable_version or None,
            "source_sha": args.source_sha or None,
            "prepared_sha": args.prepared_sha or None,
            "digests": json.loads(args.digests_json) if args.digests_json else None,
        }.items()
        if value is not None
    }
    if release_identity:
        payload["release"] = release_identity
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
    validate.add_argument("--plan-hash")
    validate.add_argument("--dispatch-nonce")
    validate.add_argument("--candidate-version")
    validate.add_argument("--stable-version")
    validate.add_argument("--source-sha")
    validate.add_argument("--prepared-sha")
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
    write.add_argument("--release-intent-id")
    write.add_argument("--candidate-id")
    write.add_argument("--attempt-id")
    write.add_argument("--plan-hash")
    write.add_argument("--dispatch-nonce")
    write.add_argument("--candidate-version")
    write.add_argument("--stable-version")
    write.add_argument("--source-sha")
    write.add_argument("--prepared-sha")
    write.add_argument("--digests-json")
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
