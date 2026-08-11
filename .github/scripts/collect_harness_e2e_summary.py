#!/usr/bin/env python3
"""Build the privacy-safe Harness summary consumed by Release Control v3."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


TECHNICAL_STATUSES = {
    "subject_error",
    "judge_error",
    "resource_limit",
    "infrastructure_error",
}


class SummaryError(ValueError):
    pass


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise SummaryError(f"cannot decode {path}: {error}") from error


def json_list(raw: str, field: str) -> list[Any]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise SummaryError(f"{field} is invalid JSON: {error}") from error
    if not isinstance(value, list) or not value:
        raise SummaryError(f"{field} must be a non-empty JSON array")
    return value


def string_list(raw: str, field: str) -> list[str]:
    value = json_list(raw, field)
    if any(not isinstance(item, str) or not item for item in value):
        raise SummaryError(f"{field} entries must be non-empty strings")
    if len(value) != len(set(value)):
        raise SummaryError(f"{field} entries must be unique")
    return value


def string_map(raw: str, field: str) -> dict[str, str]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise SummaryError(f"{field} is invalid JSON: {error}") from error
    if not isinstance(value, dict) or any(not isinstance(key, str) or not isinstance(item, str) for key, item in value.items()):
        raise SummaryError(f"{field} must be a string-to-string JSON object")
    return value


def subjects(raw: str) -> list[dict[str, str]]:
    value = json_list(raw, "subjects")
    parsed: list[dict[str, str]] = []
    for item in value:
        if not isinstance(item, dict):
            raise SummaryError("subjects entries must be objects")
        subject = {key: item.get(key) for key in ("id", "model", "provider")}
        if any(not isinstance(entry, str) or not entry for entry in subject.values()):
            raise SummaryError("subjects require non-empty id, model, and provider")
        parsed.append(subject)  # type: ignore[arg-type]
    if len({item["id"] for item in parsed}) != len(parsed):
        raise SummaryError("subject ids must be unique")
    return parsed


def optional_number(value: Any) -> int | float | None:
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) and value >= 0 else None


def metric(values: dict[str, int | float | None], *, complete_keys: tuple[str, ...]) -> dict[str, Any]:
    present = [value for value in values.values() if value is not None]
    availability = "unavailable"
    if present:
        availability = "complete" if all(values[key] is not None for key in complete_keys) else "partial"
    return {"availability": availability, **values}


def token_metric(usage: Any) -> dict[str, Any]:
    source = usage if isinstance(usage, dict) else {}
    return metric(
        {
            "input": optional_number(source.get("input_tokens")),
            "output": optional_number(source.get("output_tokens")),
            "cache_read": optional_number(source.get("cache_read_tokens")),
            "cache_write": optional_number(source.get("cache_write_tokens")),
            "reasoning": optional_number(source.get("reasoning_tokens")),
        },
        complete_keys=("input", "output"),
    )


def function_metric(metrics: Any) -> dict[str, Any]:
    totals = metrics.get("totals") if isinstance(metrics, dict) else None
    totals = totals if isinstance(totals, dict) else {}
    attempted = optional_number(totals.get("function_calls"))
    failed = optional_number(totals.get("function_call_errors"))
    succeeded = attempted - failed if isinstance(attempted, int) and isinstance(failed, int) and failed <= attempted else None
    return metric(
        {"attempted": attempted, "succeeded": succeeded, "failed": failed, "retried": None},
        complete_keys=("attempted", "succeeded", "failed", "retried"),
    )


def cost_metric(cost: Any) -> dict[str, Any]:
    source = cost if isinstance(cost, dict) else {}
    values = {
        "subject_usd": optional_number(source.get("subject_usd")),
        "judge_usd": optional_number(source.get("judge_usd")),
        "total_usd": optional_number(source.get("total_usd")),
    }
    availability = "partial" if any(value is not None for value in values.values()) else "unavailable"
    return {
        "availability": availability,
        **values,
        "origin": "reported" if availability != "unavailable" else None,
        "pricing_catalog_version": None,
    }


def sample_status(raw: str) -> str:
    if raw == "passed":
        return "passed"
    if raw == "hard_gate_failed":
        return "hard_gate_failed"
    if raw in TECHNICAL_STATUSES:
        return "technical_failed"
    raise SummaryError(f"unsupported Harness sample status: {raw}")


def contract_fingerprint(scenario: dict[str, Any]) -> str:
    contract = {
        "execution_policy": scenario.get("execution_policy"),
        "scenario_id": scenario.get("scenario_id"),
        "scenario_version": scenario.get("scenario_version", 1),
    }
    encoded = json.dumps(contract, separators=(",", ":"), sort_keys=True).encode()
    return f"sha256:{hashlib.sha256(encoded).hexdigest()}"


def error_detail(attempt: dict[str, Any], status: str) -> dict[str, str] | None:
    if status != "technical_failed":
        return None
    failures = attempt.get("failures")
    failure = failures[0] if isinstance(failures, list) and failures and isinstance(failures[0], dict) else {}
    phase = failure.get("phase") if isinstance(failure.get("phase"), str) else "execute"
    return {"category": str(attempt.get("status") or "technical"), "phase": phase}


def canonical_sample(
    attempt: dict[str, Any],
    *,
    subject: dict[str, str],
    scenario: dict[str, Any],
    repetition: int,
    attempt_number: int,
    terminal: bool,
) -> dict[str, Any]:
    status = sample_status(str(attempt.get("status", "")))
    hard_gates = attempt.get("hard_gates") if isinstance(attempt.get("hard_gates"), list) else []
    gates = [
        {"id": gate["id"], "passed": gate["passed"]}
        for gate in hard_gates
        if isinstance(gate, dict) and isinstance(gate.get("id"), str) and isinstance(gate.get("passed"), bool)
    ]
    if status == "hard_gate_failed" and not any(not gate["passed"] for gate in gates):
        raise SummaryError(f"hard_gate_failed sample has no failed gate for {scenario.get('scenario_id')}")
    metrics = attempt.get("metrics") if isinstance(attempt.get("metrics"), dict) else {}
    totals = metrics.get("totals") if isinstance(metrics.get("totals"), dict) else {}
    return {
        "subject_id": subject["id"],
        "subject_model": subject["model"],
        "subject_provider": subject["provider"],
        "scenario_id": str(scenario["scenario_id"]),
        "scenario_version": int(scenario.get("scenario_version", 1)),
        "contract_fingerprint": contract_fingerprint(scenario),
        "repetition": repetition,
        "attempt": attempt_number,
        "terminal": terminal,
        "status": status,
        "score": optional_number(attempt.get("score")),
        "hard_gates": gates,
        "error": error_detail(attempt, status),
        "duration": {"scenario_ms": optional_number(attempt.get("wall_time_ms")), "provider_ms": None},
        "tokens": token_metric(totals),
        "judge_tokens": token_metric(attempt.get("judge_usage")),
        "functions": function_metric(metrics),
        "cost": cost_metric(attempt.get("cost")),
        "technical_retry": attempt_number > 1,
        "judge_attempts": optional_number(attempt.get("judge_attempts")),
    }


def contexts(root: Path) -> dict[tuple[str, str], Path]:
    found: dict[tuple[str, str], Path] = {}
    for path in sorted(root.rglob("benchmark-context.json")) if root.exists() else []:
        value = load_json(path)
        if not isinstance(value, dict) or not isinstance(value.get("subject_id"), str) or not isinstance(value.get("scenario_id"), str):
            raise SummaryError(f"invalid benchmark context: {path}")
        key = (value["subject_id"], value["scenario_id"])
        if key in found:
            raise SummaryError(f"duplicate result context for {key[0]}/{key[1]}")
        found[key] = path
    return found


def report_path(context: Path) -> Path | None:
    for candidate in (context.parent / "results.json", context.parent.parent / "results.json"):
        if candidate.is_file():
            return candidate
    return None


def none_value(value: str) -> str | None:
    return None if value in {"", "none"} else value


def build_summary(args: argparse.Namespace) -> dict[str, Any]:
    selected = string_list(args.scenarios_json, "scenarios_json")
    required = string_list(args.required_scenarios_json, "required_scenarios_json")
    if any(item not in selected for item in required):
        raise SummaryError("required scenarios must be selected")
    selected_subjects = subjects(args.subjects)
    stack_versions = string_map(args.stack_versions, "stack_versions")
    found = contexts(args.reports_root)
    samples: list[dict[str, Any]] = []
    received_reports = 0
    engine_revisions: set[str] = set()
    protocols: set[str] = set()

    for subject in selected_subjects:
        for scenario_id in selected:
            context = found.get((subject["id"], scenario_id))
            path = report_path(context) if context else None
            if path is None:
                continue
            report = load_json(path)
            if not isinstance(report, dict):
                raise SummaryError(f"report must be an object: {path}")
            reported_subject = report.get("subject") if isinstance(report.get("subject"), dict) else {}
            if reported_subject.get("model") != subject["model"] or reported_subject.get("provider") != subject["provider"]:
                raise SummaryError(f"subject identity mismatch in {path}")
            report_scenarios = report.get("scenarios") if isinstance(report.get("scenarios"), list) else []
            matches = [entry for entry in report_scenarios if isinstance(entry, dict) and entry.get("scenario_id") == scenario_id]
            if len(matches) != 1:
                raise SummaryError(f"expected one {scenario_id} report in {path}")
            scenario = matches[0]
            runs = scenario.get("runs") if isinstance(scenario.get("runs"), list) else []
            received_reports += 1
            for repetition, run in enumerate(runs, start=1):
                if not isinstance(run, dict):
                    raise SummaryError(f"invalid run in {path}")
                retries = run.get("retry_attempts") if isinstance(run.get("retry_attempts"), list) else []
                attempts = [*retries, run]
                for attempt_number, attempt in enumerate(attempts, start=1):
                    if not isinstance(attempt, dict):
                        raise SummaryError(f"invalid retry in {path}")
                    samples.append(
                        canonical_sample(
                            attempt,
                            subject=subject,
                            scenario=scenario,
                            repetition=repetition,
                            attempt_number=attempt_number,
                            terminal=attempt_number == len(attempts),
                        )
                    )
            revision = report.get("engine_revision")
            if isinstance(revision, str) and revision:
                engine_revisions.add(revision)
            protocol = report.get("judge_protocol")
            if isinstance(protocol, str) and protocol:
                protocols.add(protocol)

    expected_reports = len(selected_subjects) * len(selected)
    expected_samples = expected_reports * args.requested_runs
    terminal = [sample for sample in samples if sample["terminal"]]
    received_samples = len(terminal)
    complete = received_reports == expected_reports and received_samples == expected_samples
    availability = "complete" if complete else "partial" if received_reports or received_samples else "unavailable"
    technical = any(sample["status"] == "technical_failed" for sample in terminal)
    hard_gate = any(any(not gate["passed"] for gate in sample["hard_gates"]) for sample in samples)
    if not complete:
        status = "incomplete"
    elif technical:
        status = "technical_failed"
    elif hard_gate:
        status = "hard_gate_failed"
    elif any(sample["status"] != "passed" for sample in terminal):
        status = "quality_advisory"
    else:
        status = "passed"
    first = next((sample for sample in samples if sample["error"]), None)
    if first is None:
        first = next((sample for sample in samples if any(not gate["passed"] for gate in sample["hard_gates"])), None)
    first_failure = None
    if first:
        failed_gate = next((gate for gate in first["hard_gates"] if not gate["passed"]), None)
        first_failure = {
            "category": first["error"]["category"] if first["error"] else "hard_gate",
            "phase": first["error"]["phase"] if first["error"] else "evaluate",
            "subject_id": first["subject_id"],
            "scenario_id": first["scenario_id"],
            "repetition": first["repetition"],
            "attempt": first["attempt"],
            **({"gate_id": failed_gate["id"]} if failed_gate else {}),
            "html_url": args.workflow_url,
        }
    generated_at = datetime.now(timezone.utc).isoformat()
    wall_time = sum(sample["duration"]["scenario_ms"] or 0 for sample in samples)
    return {
        "schema_version": 1,
        "collector_version": "workers-release-v3/1",
        "generated_at": generated_at,
        "status": status,
        "data_availability": availability,
        "execution": {
            "run_id": args.run_id,
            "attempt": args.run_attempt,
            "event": args.event,
            "actor": args.actor,
            "workflow_url": args.workflow_url,
            "workflow_sha": args.workflow_sha,
            "workflow_ref": args.workflow_ref,
            "started_at": None,
            "completed_at": generated_at,
            "wall_time_ms": wall_time,
        },
        "lane": args.lane,
        "source": {"sha": args.source_sha, "ref": args.source_ref, "repository": args.repository},
        "release": {
            "worker": none_value(args.release_worker),
            "version": none_value(args.release_version),
            "tag": none_value(args.release_tag),
            "url": None,
            "registry_tag": none_value(args.registry_tag),
            "stack_versions": stack_versions,
        },
        "profile": {
            "name": args.profile,
            "digest": args.profile_digest,
            "catalog_sha": args.catalog_sha,
            "selected_scenarios": selected,
            "required_scenarios": required,
        },
        "requested_runs": args.requested_runs,
        "subjects": selected_subjects,
        "judge": {
            "model": none_value(args.judge_model),
            "provider": none_value(args.judge_provider),
            "protocol": next(iter(protocols)) if len(protocols) == 1 else None,
        },
        "engine_revision": next(iter(engine_revisions)) if len(engine_revisions) == 1 else None,
        "coverage": {
            "expected_reports": expected_reports,
            "received_reports": received_reports,
            "expected_samples": expected_samples,
            "received_samples": received_samples,
            "complete_samples": sum(sample["status"] != "technical_failed" for sample in terminal),
            "scored_samples": sum(sample["score"] is not None for sample in terminal),
        },
        "first_failure": first_failure,
        "samples": samples,
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--reports-root", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    result.add_argument("--subjects", required=True)
    result.add_argument("--scenarios-json", required=True)
    result.add_argument("--required-scenarios-json", required=True)
    result.add_argument("--lane", choices=("deployed", "main", "daily"), required=True)
    result.add_argument("--profile", choices=("release", "custom", "full"), required=True)
    result.add_argument("--profile-digest", required=True)
    result.add_argument("--catalog-sha", required=True)
    result.add_argument("--requested-runs", type=int, required=True)
    result.add_argument("--source-sha", required=True)
    result.add_argument("--source-ref", required=True)
    result.add_argument("--repository", required=True)
    result.add_argument("--workflow-url", required=True)
    result.add_argument("--workflow-sha", required=True)
    result.add_argument("--workflow-ref", required=True)
    result.add_argument("--release-worker", required=True)
    result.add_argument("--release-version", required=True)
    result.add_argument("--release-tag", required=True)
    result.add_argument("--registry-tag", required=True)
    result.add_argument("--stack-versions", required=True)
    result.add_argument("--judge-model", required=True)
    result.add_argument("--judge-provider", required=True)
    result.add_argument("--run-id", type=int, required=True)
    result.add_argument("--run-attempt", type=int, required=True)
    result.add_argument("--event", required=True)
    result.add_argument("--actor", required=True)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if args.requested_runs < 1 or args.run_id < 1 or args.run_attempt < 1:
            raise SummaryError("run identity and requested runs must be positive")
        for field, value in (
            ("source_sha", args.source_sha),
            ("workflow_sha", args.workflow_sha),
            ("catalog_sha", args.catalog_sha),
        ):
            if not re.fullmatch(r"[0-9a-f]{40}", value):
                raise SummaryError(f"{field} must be a full lowercase commit SHA")
        if not re.fullmatch(r"[0-9a-f]{64}", args.profile_digest):
            raise SummaryError("profile_digest must be a lowercase SHA-256")
        if args.repository != "iii-hq/workers":
            raise SummaryError("repository must be iii-hq/workers")
        for field, value in (
            ("source_ref", args.source_ref),
            ("workflow_ref", args.workflow_ref),
            ("event", args.event),
            ("actor", args.actor),
        ):
            if not value:
                raise SummaryError(f"{field} must be non-empty")
        if not args.workflow_url.startswith("https://github.com/iii-hq/workers/actions/runs/"):
            raise SummaryError("workflow_url must identify an iii-hq/workers Actions run")
        summary = build_summary(args)
    except SummaryError as error:
        raise SystemExit(str(error)) from error
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
