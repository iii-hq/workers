#!/usr/bin/env python3
"""Release Control authorization and byte-exact execution evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from uuid import UUID


PHASES = {
    "prepare",
    "candidate_publish",
    "candidate_smoke",
    "stable_publish",
    "image_alias",
    "finalize",
    "verify",
}
PHASE_WORKFLOWS = {phase: f"release-{phase.replace('_', '-')}.yml" for phase in PHASES}
PHASE_WORKFLOWS.update({
    "candidate_publish": "release-candidate-publish.yml",
    "candidate_smoke": "release-candidate-smoke.yml",
    "stable_publish": "release-stable-publish.yml",
    "image_alias": "release-image-alias.yml",
})
WORKER_RE = re.compile(r"[a-z0-9][a-z0-9_-]*")
WORKFLOW_RE = re.compile(r"(?:\.github/workflows/)?[a-zA-Z0-9_.-]+\.ya?ml")
IDENTITY_FIELDS = {
    "operation_id",
    "step_id",
    "release_intent_id",
    "candidate_id",
    "attempt_id",
    "dispatch_nonce",
    "plan_hash",
}


def uuid_value(value: str, field: str) -> str:
    try:
        parsed = UUID(value)
    except ValueError as error:
        raise ValueError(f"{field} must be a UUID") from error
    if str(parsed) != value.lower():
        raise ValueError(f"{field} must use canonical UUID form")
    return str(parsed)


def sha256_hex(value: str, field: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError(f"{field} must be a 64-char lowercase SHA-256")
    return value


def commit_sha(value: str, field: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", value):
        raise ValueError(f"{field} must be a full lowercase commit SHA")
    return value


def json_value(value: str, field: str, expected: type) -> object:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as error:
        raise ValueError(f"{field} must be valid JSON") from error
    if not isinstance(parsed, expected):
        raise ValueError(f"{field} must be a JSON {expected.__name__}")
    return parsed


def nullable(value: str | None) -> str | None:
    return None if value in (None, "", "none", "null") else value


def validate_executor(args: argparse.Namespace) -> None:
    if args.repository != "iii-hq/workers":
        raise ValueError("Workers release executions require repository=iii-hq/workers")
    if not WORKFLOW_RE.fullmatch(args.workflow):
        raise ValueError("workflow must be a workflow filename or path")
    if args.workflow not in PHASE_WORKFLOWS.values():
        raise ValueError("workflow is not a release-train entrypoint")
    if args.run_id < 1 or args.run_attempt < 1:
        raise ValueError("run_id and run_attempt must be positive integers")
    if args.event != "workflow_dispatch":
        raise ValueError("release executions require event=workflow_dispatch")
    commit_sha(args.sha, "sha")


def validate_worker(value: str) -> str:
    if not WORKER_RE.fullmatch(value):
        raise ValueError("worker must match [a-z0-9][a-z0-9_-]*")
    return value


def identity_value(raw: str) -> dict[str, str]:
    identity = json_value(raw, "identity", dict)
    assert isinstance(identity, dict)
    if set(identity) != IDENTITY_FIELDS:
        raise ValueError(
            "identity fields differ from release contract: "
            f"missing={sorted(IDENTITY_FIELDS - set(identity))} "
            f"unknown={sorted(set(identity) - IDENTITY_FIELDS)}"
        )
    if not all(isinstance(identity[field], str) and identity[field] for field in IDENTITY_FIELDS):
        raise ValueError("identity fields must be non-empty strings")
    for field in (
        "operation_id", "step_id", "release_intent_id", "candidate_id", "attempt_id", "dispatch_nonce",
    ):
        uuid_value(identity[field], field)
    sha256_hex(identity["plan_hash"], "plan_hash")
    return identity  # type: ignore[return-value]


def validate_identity(args: argparse.Namespace) -> int:
    identity_value(args.identity)
    return 0


def validate_dispatch(args: argparse.Namespace) -> int:
    uuid_value(args.operation_id, "operation_id")
    uuid_value(args.step_id, "step_id")
    uuid_value(args.dispatch_nonce, "dispatch_nonce")
    sha256_hex(args.descriptor_sha256, "descriptor_sha256")
    if args.run_attempt is None:
        raise ValueError("GITHUB_RUN_ATTEMPT or --run-attempt is required")
    if args.mutating and args.run_attempt != 1:
        raise ValueError("mutating executors cannot be rerun; create a recovery operation")
    if args.plan_hash:
        sha256_hex(args.plan_hash, "plan_hash")
    for value, field in ((args.source_sha, "source_sha"), (args.prepared_sha, "prepared_sha")):
        if value:
            commit_sha(value, field)
    return 0


def validate_effects(raw: str) -> list[dict[str, object]]:
    effects = json_value(raw, "effects", list)
    assert isinstance(effects, list)
    allowed = {"surface", "state", "immutable_id", "before", "after"}
    for index, effect in enumerate(effects):
        if not isinstance(effect, dict):
            raise ValueError(f"effects[{index}] must be an object")
        unknown = set(effect) - allowed
        if unknown or not isinstance(effect.get("surface"), str) or not effect["surface"]:
            raise ValueError(f"effects[{index}] has invalid fields")
        if effect.get("state") not in {"absent", "present", "unknown"}:
            raise ValueError(f"effects[{index}].state is invalid")
        for field in ("immutable_id",):
            if field in effect and (not isinstance(effect[field], str) or not effect[field]):
                raise ValueError(f"effects[{index}].{field} must be a non-empty string")
    return effects


def validate_artifacts(raw: str) -> list[dict[str, object]]:
    artifacts = json_value(raw, "artifacts", list)
    assert isinstance(artifacts, list)
    required = {"name", "role", "sha256", "size"}
    for index, artifact in enumerate(artifacts):
        if not isinstance(artifact, dict) or set(artifact) != required:
            raise ValueError(f"artifacts[{index}] must contain exactly {sorted(required)}")
        if not all(isinstance(artifact[key], str) and artifact[key] for key in ("name", "role", "sha256")):
            raise ValueError(f"artifacts[{index}] string fields must be non-empty")
        sha256_hex(str(artifact["sha256"]), f"artifacts[{index}].sha256")
        if not isinstance(artifact["size"], int) or artifact["size"] < 0:
            raise ValueError(f"artifacts[{index}].size must be a non-negative integer")
    return artifacts


def validate_error(raw: str) -> dict[str, object] | None:
    value = json.loads(raw)
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("error must be null or an object")
    required = {"code", "category", "retryable", "message"}
    if set(value) != required:
        raise ValueError(f"error must contain exactly {sorted(required)}")
    if not all(isinstance(value[key], str) and value[key] for key in ("code", "category", "message")):
        raise ValueError("error string fields must be non-empty")
    if not isinstance(value["retryable"], bool):
        raise ValueError("error.retryable must be a boolean")
    return value


def write_result(args: argparse.Namespace) -> int:
    if args.phase not in PHASES:
        raise ValueError(f"unsupported release phase: {args.phase}")
    validate_executor(args)
    if args.workflow != PHASE_WORKFLOWS[args.phase]:
        raise ValueError(f"phase {args.phase} requires workflow {PHASE_WORKFLOWS[args.phase]}")
    prepared_sha = nullable(args.prepared_sha)
    candidate_version = nullable(args.candidate_version)
    stable_version = nullable(args.stable_version)
    payload = {
        "contract": "release-execution",
        "identity": {
            "operation_id": uuid_value(args.operation_id, "operation_id"),
            "step_id": uuid_value(args.step_id, "step_id"),
            "release_intent_id": uuid_value(args.release_intent_id, "release_intent_id"),
            "candidate_id": uuid_value(args.candidate_id, "candidate_id"),
            "attempt_id": uuid_value(args.attempt_id, "attempt_id"),
            "dispatch_nonce": uuid_value(args.dispatch_nonce, "dispatch_nonce"),
            "plan_hash": sha256_hex(args.plan_hash, "plan_hash"),
        },
        "executor": {
            "repository": args.repository,
            "workflow": args.workflow,
            "sha": args.sha,
            "run_id": args.run_id,
            "run_attempt": args.run_attempt,
            "event": args.event,
        },
        "subject": {
            "worker": validate_worker(args.worker),
            "phase": args.phase,
            "source_sha": commit_sha(args.source_sha, "source_sha"),
            "prepared_sha": commit_sha(prepared_sha, "prepared_sha") if prepared_sha else None,
            "candidate_version": candidate_version,
            "stable_version": stable_version,
            "descriptor_sha256": sha256_hex(args.descriptor_sha256, "descriptor_sha256"),
        },
        "outcome": args.outcome,
        "effects": validate_effects(args.effects),
        "artifacts": validate_artifacts(args.artifacts_json),
        "error": validate_error(args.error_json),
        "completed_at": args.completed_at
        or datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    }
    if args.outcome == "succeeded" and payload["error"] is not None:
        raise ValueError("succeeded results must use error=null")
    if args.outcome != "succeeded" and payload["error"] is None:
        raise ValueError("failed/canceled results require error details")
    body = json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n"
    args.output.write_bytes(body)
    return 0


def write_test_result(args: argparse.Namespace) -> int:
    """Preserve the non-release E2E evidence contract outside this cutover."""
    payload = {
        "schema_version": 1,
        "kind": args.kind,
        "repository": args.repository,
        "operation_id": uuid_value(args.operation_id, "operation_id"),
        "step_id": uuid_value(args.step_id, "step_id"),
        "run": {
            "id": args.run_id,
            "attempt": args.run_attempt,
            "workflow": args.workflow,
            "event": args.event,
            "sha": commit_sha(args.sha, "sha"),
        },
        "subject": json_value(args.subject, "subject", dict),
        "checks": json_value(args.checks, "checks", list),
        "effects": json_value(args.effects, "effects", list),
        "outputs": json_value(args.outputs, "outputs", dict),
    }
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


def oidc_token(audience: str) -> str:
    request_url = os.environ.get("ACTIONS_ID_TOKEN_REQUEST_URL", "")
    request_token = os.environ.get("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "")
    if not request_url or not request_token:
        raise ValueError("GitHub OIDC request environment is unavailable")
    separator = "&" if "?" in request_url else "?"
    request = urllib.request.Request(
        request_url + separator + urllib.parse.urlencode({"audience": audience}),
        headers={"Authorization": f"Bearer {request_token}"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        payload = json.loads(response.read())
    token = payload.get("value") if isinstance(payload, dict) else None
    if not isinstance(token, str) or not token:
        raise ValueError("GitHub OIDC response did not contain a token")
    return token


def authorize_dispatch(args: argparse.Namespace) -> int:
    validate_executor(args)
    payload = {
        "identity": {
            "operation_id": uuid_value(args.operation_id, "operation_id"),
            "step_id": uuid_value(args.step_id, "step_id"),
            "release_intent_id": uuid_value(args.release_intent_id, "release_intent_id"),
            "candidate_id": uuid_value(args.candidate_id, "candidate_id"),
            "attempt_id": uuid_value(args.attempt_id, "attempt_id"),
            "dispatch_nonce": uuid_value(args.dispatch_nonce, "dispatch_nonce"),
            "plan_hash": sha256_hex(args.plan_hash, "plan_hash"),
        },
        "executor": {
            "repository": args.repository,
            "workflow": args.workflow,
            "sha": args.sha,
            "run_id": args.run_id,
            "run_attempt": args.run_attempt,
            "event": args.event,
        },
        "subject": {
            "worker": validate_worker(args.worker),
            "source_sha": commit_sha(args.source_sha, "source_sha"),
            "descriptor_sha256": sha256_hex(args.descriptor_sha256, "descriptor_sha256"),
        },
    }
    body = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    digest = "sha256:" + hashlib.sha256(body).hexdigest()
    request = urllib.request.Request(
        args.api_url.rstrip("/") + "/api/executor-dispatches/authorize",
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {oidc_token(args.audience)}",
            "Content-Type": "application/json",
            "x-release-result-sha256": digest,
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        if not 200 <= response.status < 300:
            raise ValueError(f"Release Control rejected dispatch with HTTP {response.status}")
    return 0


def post_result(args: argparse.Namespace) -> int:
    body = args.result.read_bytes()
    payload = json.loads(body)
    if not isinstance(payload, dict) or payload.get("contract") != "release-execution":
        raise ValueError("release result has the wrong contract")
    digest = "sha256:" + hashlib.sha256(body).hexdigest()
    request = urllib.request.Request(
        args.api_url.rstrip("/") + "/api/executor-results",
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {oidc_token(args.audience)}",
            "Content-Type": "application/json",
            "x-release-result-sha256": digest,
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        if not 200 <= response.status < 300:
            raise ValueError(f"Release Control rejected result with HTTP {response.status}")
    print(digest)
    return 0


def add_executor_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repository", required=True)
    parser.add_argument("--operation-id", required=True)
    parser.add_argument("--step-id", required=True)
    parser.add_argument("--run-id", type=int, required=True)
    parser.add_argument("--run-attempt", type=int, required=True)
    parser.add_argument("--workflow", required=True)
    parser.add_argument("--event", required=True)
    parser.add_argument("--sha", required=True)


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)

    identity = commands.add_parser("validate-identity")
    identity.add_argument("--identity", required=True)
    identity.set_defaults(handler=validate_identity)

    validate = commands.add_parser("validate-dispatch")
    validate.add_argument("--operation-id", required=True)
    validate.add_argument("--step-id", required=True)
    validate.add_argument("--dispatch-nonce", required=True)
    validate.add_argument("--descriptor-sha256", required=True)
    validate.add_argument("--run-attempt", type=int, default=os.environ.get("GITHUB_RUN_ATTEMPT"))
    validate.add_argument("--mutating", action="store_true")
    validate.add_argument("--plan-hash")
    validate.add_argument("--source-sha")
    validate.add_argument("--prepared-sha")
    validate.set_defaults(handler=validate_dispatch)

    write = commands.add_parser("write-result")
    add_executor_args(write)
    write.add_argument("--release-intent-id", required=True)
    write.add_argument("--candidate-id", required=True)
    write.add_argument("--attempt-id", required=True)
    write.add_argument("--dispatch-nonce", required=True)
    write.add_argument("--plan-hash", required=True)
    write.add_argument("--worker", required=True)
    write.add_argument("--phase", choices=sorted(PHASES), required=True)
    write.add_argument("--source-sha", required=True)
    write.add_argument("--prepared-sha", default="none")
    write.add_argument("--candidate-version", default="none")
    write.add_argument("--stable-version", default="none")
    write.add_argument("--descriptor-sha256", required=True)
    write.add_argument("--outcome", choices=["succeeded", "failed", "canceled"], required=True)
    write.add_argument("--effects", required=True)
    write.add_argument("--artifacts-json", required=True)
    write.add_argument("--error-json", required=True)
    write.add_argument("--completed-at")
    write.add_argument("--output", type=Path, required=True)
    write.set_defaults(handler=write_result)

    test = commands.add_parser("write-test-result")
    add_executor_args(test)
    test.add_argument("--kind", required=True)
    test.add_argument("--subject", required=True)
    test.add_argument("--checks", required=True)
    test.add_argument("--effects", required=True)
    test.add_argument("--outputs", required=True)
    test.add_argument("--output", type=Path, required=True)
    test.set_defaults(handler=write_test_result)

    authorize = commands.add_parser("authorize-dispatch")
    authorize.add_argument("--api-url", required=True)
    authorize.add_argument("--audience", default="release-control-workers")
    add_executor_args(authorize)
    authorize.add_argument("--release-intent-id", required=True)
    authorize.add_argument("--candidate-id", required=True)
    authorize.add_argument("--attempt-id", required=True)
    authorize.add_argument("--dispatch-nonce", required=True)
    authorize.add_argument("--plan-hash", required=True)
    authorize.add_argument("--worker", required=True)
    authorize.add_argument("--source-sha", required=True)
    authorize.add_argument("--descriptor-sha256", required=True)
    authorize.set_defaults(handler=authorize_dispatch)

    post = commands.add_parser("post-result")
    post.add_argument("--result", type=Path, required=True)
    post.add_argument("--api-url", required=True)
    post.add_argument("--audience", default="release-control-workers")
    post.set_defaults(handler=post_result)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        return args.handler(args)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
