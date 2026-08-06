#!/usr/bin/env python3
"""Update the static Harness E2E execution manifest and retained reports."""

from __future__ import annotations

import argparse
import json
from datetime import datetime
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 3
MANIFEST_PREFIX = "window.HARNESS_EXECUTIONS = "
FAILURE_CONCLUSIONS = {
    "action_required",
    "failure",
    "startup_failure",
    "stale",
    "timed_out",
}


class PublishError(ValueError):
    """Raised when dashboard publication inputs are unsafe or malformed."""


def load_json(path: Path | None) -> dict[str, Any] | None:
    if path is None or not path.is_file():
        return None
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise PublishError(f"cannot decode {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise PublishError(f"{path} must contain an object")
    return value


def load_manifest(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {"schema_version": SCHEMA_VERSION, "executions": []}
    text = path.read_text().strip()
    if not text.startswith(MANIFEST_PREFIX) or not text.endswith(";"):
        raise PublishError(f"{path} is not a Harness execution manifest")
    try:
        value = json.loads(text[len(MANIFEST_PREFIX) : -1])
    except json.JSONDecodeError as exc:
        raise PublishError(f"cannot decode {path}: {exc}") from exc
    if not isinstance(value, dict) or not isinstance(value.get("executions"), list):
        raise PublishError(f"{path} has an invalid execution manifest")
    return value


def validate_artifact_identity(
    value: dict[str, Any] | None,
    metadata: dict[str, Any],
    *,
    label: str,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    if value is None:
        return None, None
    execution = value.get("execution")
    if not isinstance(execution, dict):
        return None, {
            "kind": "artifact_identity",
            "message": f"Ignored {label}: execution identity is missing",
        }
    actual_run_id = str(execution.get("run_id") or "")
    actual_attempt = int(
        optional_number(execution.get("attempt") or execution.get("run_attempt"))
        or 0
    )
    expected_run_id = str(metadata["run_id"])
    expected_attempt = int(metadata["attempt"])
    if actual_run_id == expected_run_id and actual_attempt == expected_attempt:
        return value, None
    actual = f"{actual_run_id or 'unknown'} attempt {actual_attempt or 'unknown'}"
    expected = f"{expected_run_id} attempt {expected_attempt}"
    return None, {
        "kind": "artifact_identity",
        "message": f"Ignored {label} from {actual}; expected {expected}",
    }


def optional_number(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return float(value)


def sum_complete(values: list[float | None]) -> float | None:
    return sum(value for value in values if value is not None) if values and all(
        value is not None for value in values
    ) else None


def mean_available(values: list[float | None]) -> tuple[float | None, int]:
    available = [value for value in values if value is not None]
    if not available:
        return None, 0
    return sum(available) / len(available), len(available)


def run_metric(run: dict[str, Any], metric_id: str) -> float | None:
    metrics = run.get("metrics", {})
    totals = metrics.get("totals", {}) if isinstance(metrics, dict) else {}
    if not isinstance(totals, dict):
        totals = {}
    if metric_id == "tokens":
        input_tokens = optional_number(totals.get("input_tokens"))
        output_tokens = optional_number(totals.get("output_tokens"))
        if input_tokens is None or output_tokens is None:
            return None
        return input_tokens + output_tokens
    if metric_id == "duration_seconds":
        wall_time_ms = optional_number(run.get("wall_time_ms"))
        return wall_time_ms / 1000 if wall_time_ms is not None else None
    if metric_id == "cost_usd":
        cost = run.get("cost", {})
        return (
            optional_number(cost.get("total_usd"))
            if isinstance(cost, dict)
            else None
        )
    return optional_number(totals.get(metric_id))


def scenario_contract(
    scenario: dict[str, Any],
    scenario_id: str,
    runs: list[dict[str, Any]],
) -> dict[str, Any]:
    execution_policy = scenario.get("execution_policy", {})
    if not isinstance(execution_policy, dict):
        execution_policy = {}
    threshold = optional_number(scenario.get("threshold"))
    return {
        "execution_policy": execution_policy,
        "scenario_id": scenario_id,
        "scenario_version": int(
            optional_number(scenario.get("scenario_version")) or 1
        ),
        "threshold": int(threshold) if threshold is not None else None,
    }


def contract_fingerprint(contract: dict[str, Any]) -> str:
    canonical = json.dumps(
        contract,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    value = 2_166_136_261
    for byte in canonical:
        value ^= byte
        value = (value * 16_777_619) & 0xFFFF_FFFF
    return f"fnv1a32:{value:08x}"


def build_scenario_metrics(detail: dict[str, Any]) -> list[dict[str, Any]]:
    metric_ids = (
        "tokens",
        "duration_seconds",
        "cost_usd",
        "function_calls",
        "function_call_errors",
        "sessions",
        "turns",
    )
    grouped: dict[tuple[str, str], dict[str, Any]] = {}
    reports = detail.get("reports", [])
    if not isinstance(reports, list):
        return []
    for report_entry in reports:
        if not isinstance(report_entry, dict) or not report_entry.get("available"):
            continue
        report = report_entry.get("report", {})
        report_scenarios = report.get("scenarios", []) if isinstance(report, dict) else []
        if not isinstance(report_scenarios, list):
            continue
        for scenario in report_scenarios:
            if not isinstance(scenario, dict):
                continue
            scenario_id = str(
                scenario.get("scenario_id")
                or report_entry.get("scenario_id")
                or ""
            )
            if not scenario_id:
                continue
            subject_id = str(report_entry.get("subject_id") or "")
            runs = scenario.get("runs", [])
            if not isinstance(runs, list):
                continue
            key = (subject_id, scenario_id)
            entry = grouped.setdefault(
                key,
                {
                    "runs": [],
                    "scenario": scenario,
                },
            )
            entry["runs"].extend(run for run in runs if isinstance(run, dict))

    result = []
    for (subject_id, scenario_id), entry in sorted(grouped.items()):
        runs = entry["runs"]
        contract = scenario_contract(entry["scenario"], scenario_id, runs)
        averages: dict[str, float | None] = {}
        samples: dict[str, int] = {}
        for metric_id in metric_ids:
            average, sample_count = mean_available(
                [run_metric(run, metric_id) for run in runs]
            )
            averages[metric_id] = average
            samples[metric_id] = sample_count
        result.append(
            {
                "subject_id": subject_id,
                "scenario_id": scenario_id,
                "scenario_version": contract["scenario_version"],
                "contract_fingerprint": contract_fingerprint(contract),
                "run_count": len(runs),
                "averages": averages,
                "samples": samples,
            }
        )
    return result


def build_execution_efficiency_totals(detail: dict[str, Any]) -> dict[str, float | None]:
    tokens: list[float | None] = []
    function_calls: list[float | None] = []
    reports = detail.get("reports", [])
    if not isinstance(reports, list):
        reports = []
    for report_entry in reports:
        if not isinstance(report_entry, dict) or not report_entry.get("available"):
            continue
        report = report_entry.get("report", {})
        scenarios = report.get("scenarios", []) if isinstance(report, dict) else []
        if not isinstance(scenarios, list):
            continue
        for scenario in scenarios:
            runs = scenario.get("runs", []) if isinstance(scenario, dict) else []
            if not isinstance(runs, list):
                continue
            for run in runs:
                if not isinstance(run, dict):
                    continue
                tokens.append(run_metric(run, "tokens"))
                function_calls.append(run_metric(run, "function_calls"))
    return {
        "total_tokens": sum_complete(tokens),
        "function_calls": sum_complete(function_calls),
    }


def _pick(value: Any, keys: tuple[str, ...]) -> dict[str, Any]:
    source = value if isinstance(value, dict) else {}
    return {key: source[key] for key in keys if key in source}


def _public_failures(value: Any) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        return []
    return [
        _pick(failure, ("phase", "message"))
        for failure in value
        if isinstance(failure, dict)
    ]


def _public_gates(value: Any) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        return []
    return [
        _pick(gate, ("id", "passed", "reason"))
        for gate in value
        if isinstance(gate, dict)
    ]


def _public_metrics(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    totals = _pick(
        value.get("totals"),
        (
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "reasoning_tokens",
            "function_calls",
            "function_call_errors",
            "sessions",
            "turns",
            "cost_usd",
        ),
    )
    return {"totals": totals} if totals else None


def _public_retry(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    retry = _pick(value, ("run_id", "session_id", "wall_time_ms", "status"))
    retry["cost"] = _pick(value.get("cost"), ("subject_usd", "judge_usd", "total_usd"))
    retry["failures"] = _public_failures(value.get("failures"))
    return retry


def _public_run(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    run = _pick(
        value,
        (
            "run_id",
            "session_id",
            "wall_time_ms",
            "score",
            "status",
            "judge_attempts",
        ),
    )
    metrics = _public_metrics(value.get("metrics"))
    if metrics is not None:
        run["metrics"] = metrics
    run["cost"] = _pick(value.get("cost"), ("subject_usd", "judge_usd", "total_usd"))
    run["hard_gates"] = _public_gates(value.get("hard_gates"))
    run["failures"] = _public_failures(value.get("failures"))
    retries = [
        retry
        for item in value.get("retry_attempts", [])
        if (retry := _public_retry(item)) is not None
    ] if isinstance(value.get("retry_attempts"), list) else []
    run["retry_attempts"] = retries
    return run


def _public_scenario(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    scenario = _pick(
        value,
        ("scenario_id", "scenario_version", "passed", "threshold"),
    )
    aggregate = _pick(
        value.get("aggregate"),
        (
            "runs",
            "scored_runs",
            "passed_runs",
            "required_passes",
            "pass_rate",
            "median_score",
            "hard_gate_failures",
            "technical_failures",
        ),
    )
    raw_aggregate = value.get("aggregate")
    if isinstance(raw_aggregate, dict):
        aggregate["cost"] = _pick(
            raw_aggregate.get("cost"),
            ("subject_usd", "judge_usd", "total_usd"),
        )
    scenario["aggregate"] = aggregate
    scenario["runs"] = [
        run
        for item in value.get("runs", [])
        if (run := _public_run(item)) is not None
    ] if isinstance(value.get("runs"), list) else []
    return scenario


def _public_subject_summary(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    subject = _pick(
        value,
        (
            "id",
            "model",
            "provider",
            "engine_revision",
            "passed",
            "expected_reports",
            "received_reports",
            "scenario_pass_rate",
            "report_coverage",
            "hard_gate_failures",
            "technical_failures",
            "retry_attempts",
            "total_cost_usd",
            "wall_time_seconds",
        ),
    )
    subject["judge"] = _pick(value.get("judge"), ("model", "provider", "protocol"))
    subject["scenarios"] = [
        _pick(
            scenario,
            (
                "id",
                "status",
                "passed",
                "threshold",
                "runs",
                "median_score",
                "pass_rate",
                "hard_gate_failures",
                "technical_failures",
                "retries",
                "total_cost_usd",
                "wall_time_seconds",
            ),
        )
        for scenario in value.get("scenarios", [])
        if isinstance(scenario, dict)
    ] if isinstance(value.get("scenarios"), list) else []
    return subject


def compact_public_detail(
    detail: dict[str, Any],
    metadata: dict[str, Any],
) -> dict[str, Any]:
    """Project raw execution evidence into the diagnostics-safe Pages contract."""
    public_metadata = _pick(
        metadata,
        (
            "id",
            "run_id",
            "attempt",
            "workflow_name",
            "workflow_url",
            "event",
            "actor",
            "started_at",
            "completed_at",
            "conclusion",
            "head_sha",
            "head_branch",
            "repository",
        ),
    )
    public = {
        "schema_version": SCHEMA_VERSION,
        "execution": {
            **_pick(
                detail.get("execution"),
                ("id", "run_id", "attempt", "event", "actor", "workflow_url"),
            ),
            **public_metadata,
        },
        "generated_at": str(detail.get("generated_at") or ""),
        "lane": str(detail.get("lane") or "daily"),
        "source": _pick(detail.get("source"), ("sha", "ref", "repository")),
        "workflow_url": str(detail.get("workflow_url") or metadata.get("workflow_url") or ""),
        "release": _pick(
            detail.get("release"),
            ("tag", "worker", "version", "url", "registry_tag"),
        ),
        "requested_runs": detail.get("requested_runs"),
        "subjects": [
            subject
            for item in detail.get("subjects", [])
            if (subject := _public_subject_summary(item)) is not None
        ] if isinstance(detail.get("subjects"), list) else [],
        "reports": [],
    }
    reports = detail.get("reports", [])
    if not isinstance(reports, list):
        return public
    for record in reports:
        if not isinstance(record, dict):
            continue
        public_record = _pick(record, ("subject_id", "scenario_id", "available"))
        report = record.get("report")
        if isinstance(report, dict):
            public_record["report"] = {
                "subject": _pick(report.get("subject"), ("model", "provider")),
                "judge": _pick(report.get("judge"), ("model", "provider", "protocol")),
                "engine_revision": report.get("engine_revision"),
                "passed": report.get("passed"),
                "scenarios": [
                    scenario
                    for item in report.get("scenarios", [])
                    if (scenario := _public_scenario(item)) is not None
                ] if isinstance(report.get("scenarios"), list) else [],
            }
        else:
            public_record["report"] = None
        public["reports"].append(public_record)
    return public


def elapsed_seconds(started_at: str, completed_at: str) -> float | None:
    if not started_at or not completed_at:
        return None
    try:
        start = datetime.fromisoformat(started_at.replace("Z", "+00:00"))
        end = datetime.fromisoformat(completed_at.replace("Z", "+00:00"))
    except ValueError:
        return None
    return max(0.0, (end - start).total_seconds())


def execution_status(
    conclusion: str,
    subjects: list[dict[str, Any]],
    expected_reports: int,
    received_reports: int,
    hard_gate_failures: int,
    technical_failures: int,
    *,
    has_failed_job: bool = False,
) -> str:
    if conclusion == "cancelled":
        return "cancelled"
    if not conclusion:
        return "running"
    if not subjects or expected_reports == 0:
        if conclusion in FAILURE_CONCLUSIONS or has_failed_job:
            return "infra_failed"
        return "incomplete"
    if received_reports < expected_reports:
        return "incomplete"
    if technical_failures:
        return "technical_failed"
    if hard_gate_failures:
        return "hard_gate_failed"
    if conclusion != "success" or has_failed_job:
        return "infra_failed"
    if not all(bool(subject.get("passed")) for subject in subjects):
        return "quality_advisory"
    return "passed"


def _compact_message(value: Any, fallback: str) -> str:
    message = " ".join(str(value or "").split()) or fallback
    return message[:500]


def failed_job_diagnostic(jobs_document: dict[str, Any] | None) -> dict[str, Any] | None:
    jobs = jobs_document.get("jobs", []) if jobs_document else []
    if not isinstance(jobs, list):
        return None
    failed_jobs = [
        job
        for job in jobs
        if isinstance(job, dict) and job.get("conclusion") in FAILURE_CONCLUSIONS
    ]
    failed_jobs.sort(key=lambda job: (str(job.get("started_at") or ""), str(job.get("name") or "")))
    if not failed_jobs:
        return None
    job = failed_jobs[0]
    steps = job.get("steps", [])
    failed_steps = [
        step
        for step in steps
        if isinstance(step, dict) and step.get("conclusion") in FAILURE_CONCLUSIONS
    ] if isinstance(steps, list) else []
    failed_steps.sort(
        key=lambda step: (
            int(optional_number(step.get("number")) or 0),
            str(step.get("name") or ""),
        )
    )
    step = failed_steps[0] if failed_steps else None
    job_name = str(job.get("name") or "workflow job")
    step_name = str(step.get("name") or "") if step else ""
    return {
        "kind": "job",
        "job_name": job_name,
        "step_name": step_name,
        "message": _compact_message(
            f"{job_name}: {step_name}" if step_name else f"{job_name} failed",
            "Workflow job failed",
        ),
        "url": str(job.get("html_url") or ""),
    }


def _report_scenarios(detail: dict[str, Any] | None) -> list[tuple[str, str, dict[str, Any]]]:
    reports = detail.get("reports", []) if detail else []
    if not isinstance(reports, list):
        return []
    result = []
    for report_entry in reports:
        if not isinstance(report_entry, dict) or not report_entry.get("available"):
            continue
        report = report_entry.get("report", {})
        scenarios = report.get("scenarios", []) if isinstance(report, dict) else []
        if not isinstance(scenarios, list):
            continue
        for scenario in scenarios:
            if isinstance(scenario, dict):
                result.append(
                    (
                        str(report_entry.get("subject_id") or ""),
                        str(scenario.get("scenario_id") or report_entry.get("scenario_id") or ""),
                        scenario,
                    )
                )
    return result


def report_diagnostic(
    status: str,
    snapshot: dict[str, Any] | None,
    detail: dict[str, Any] | None,
) -> dict[str, Any] | None:
    if status == "incomplete" and snapshot:
        for subject in snapshot.get("subjects", []):
            if not isinstance(subject, dict):
                continue
            for scenario in subject.get("scenarios", []):
                if isinstance(scenario, dict) and scenario.get("status") == "missing_report":
                    subject_id = str(subject.get("id") or "")
                    scenario_id = str(scenario.get("id") or "")
                    return {
                        "kind": "missing_report",
                        "subject_id": subject_id,
                        "scenario_id": scenario_id,
                        "message": f"Missing report for {subject_id}/{scenario_id}",
                    }

    if status == "technical_failed":
        for subject_id, scenario_id, scenario in _report_scenarios(detail):
            runs = scenario.get("runs", [])
            if not isinstance(runs, list):
                continue
            for run in runs:
                failures = run.get("failures", []) if isinstance(run, dict) else []
                if not isinstance(failures, list) or not failures:
                    continue
                failure = next((item for item in failures if isinstance(item, dict)), None)
                if failure:
                    return {
                        "kind": "technical",
                        "subject_id": subject_id,
                        "scenario_id": scenario_id,
                        "phase": str(failure.get("phase") or "execute"),
                        "message": _compact_message(
                            failure.get("message"), "Technical execution failure"
                        ),
                    }

    if status == "hard_gate_failed":
        for subject_id, scenario_id, scenario in _report_scenarios(detail):
            runs = scenario.get("runs", [])
            if not isinstance(runs, list):
                continue
            for run in runs:
                gates = run.get("hard_gates", []) if isinstance(run, dict) else []
                if not isinstance(gates, list):
                    continue
                gate = next(
                    (
                        item
                        for item in gates
                        if isinstance(item, dict) and not item.get("passed", False)
                    ),
                    None,
                )
                if gate:
                    return {
                        "kind": "hard_gate",
                        "subject_id": subject_id,
                        "scenario_id": scenario_id,
                        "id": str(gate.get("id") or ""),
                        "message": _compact_message(gate.get("reason"), "Hard gate failed"),
                    }

    if status == "quality_advisory" and snapshot:
        for subject in snapshot.get("subjects", []):
            if not isinstance(subject, dict):
                continue
            for scenario in subject.get("scenarios", []):
                if not isinstance(scenario, dict) or scenario.get("passed"):
                    continue
                score = optional_number(scenario.get("median_score"))
                threshold = optional_number(scenario.get("threshold"))
                description = "Quality result did not meet the scenario threshold"
                if score is not None and threshold is not None:
                    description = f"Median score {score:g} is below threshold {threshold:g}"
                return {
                    "kind": "quality",
                    "subject_id": str(subject.get("id") or ""),
                    "scenario_id": str(scenario.get("id") or ""),
                    "message": description,
                }
    return None


def build_summary(
    snapshot: dict[str, Any] | None,
    metadata: dict[str, Any],
    *,
    detail: dict[str, Any] | None = None,
    jobs_document: dict[str, Any] | None = None,
    artifact_failure: dict[str, Any] | None = None,
) -> dict[str, Any]:
    subjects = snapshot.get("subjects", []) if snapshot else []
    if not isinstance(subjects, list):
        subjects = []
    valid_subjects = [subject for subject in subjects if isinstance(subject, dict)]
    scenarios = [
        scenario
        for subject in valid_subjects
        for scenario in subject.get("scenarios", [])
        if isinstance(scenario, dict)
    ]
    scores = [
        score
        for scenario in scenarios
        if (score := optional_number(scenario.get("median_score"))) is not None
    ]
    expected_reports = sum(
        int(optional_number(subject.get("expected_reports")) or 0)
        for subject in valid_subjects
    )
    received_reports = sum(
        int(optional_number(subject.get("received_reports")) or 0)
        for subject in valid_subjects
    )
    passed_scenarios = sum(bool(scenario.get("passed")) for scenario in scenarios)
    subject_costs = [
        optional_number(subject.get("total_cost_usd")) for subject in valid_subjects
    ]
    subject_wall_times = [
        optional_number(subject.get("wall_time_seconds")) for subject in valid_subjects
    ]
    hard_gate_failures = sum(
        int(optional_number(subject.get("hard_gate_failures")) or 0)
        for subject in valid_subjects
    )
    technical_failures = sum(
        int(optional_number(subject.get("technical_failures")) or 0)
        for subject in valid_subjects
    )
    retries = sum(
        int(optional_number(subject.get("retry_attempts")) or 0)
        for subject in valid_subjects
    )
    missing_reports = max(0, expected_reports - received_reports)
    conclusion = metadata["conclusion"]
    job_failure = failed_job_diagnostic(jobs_document)
    status = execution_status(
        conclusion,
        valid_subjects,
        expected_reports,
        received_reports,
        hard_gate_failures,
        technical_failures,
        has_failed_job=job_failure is not None,
    )
    first_failure = (
        job_failure or artifact_failure
        if status == "infra_failed"
        else report_diagnostic(status, snapshot, detail)
        or artifact_failure
        or job_failure
    )
    snapshot_execution = snapshot.get("execution", {}) if snapshot else {}
    if not isinstance(snapshot_execution, dict):
        snapshot_execution = {}

    return {
        "id": metadata["id"],
        "run_id": metadata["run_id"],
        "attempt": metadata["attempt"],
        "workflow_name": metadata["workflow_name"],
        "workflow_url": metadata["workflow_url"],
        "event": metadata["event"],
        "actor": metadata["actor"],
        "started_at": metadata["started_at"],
        "completed_at": metadata["completed_at"],
        "conclusion": conclusion,
        "status": status,
        "first_failure": first_failure,
        "workflow_duration_seconds": elapsed_seconds(
            metadata["started_at"], metadata["completed_at"]
        ),
        "availability": "aggregate" if snapshot else "unavailable",
        "detail_path": None,
        "generated_at": snapshot.get("generated_at", "") if snapshot else "",
        "lane": snapshot.get("lane", "daily") if snapshot else "daily",
        "source": (
            snapshot.get("source", {})
            if snapshot
            else {
                "sha": metadata["head_sha"],
                "ref": metadata["head_branch"],
                "repository": metadata["repository"],
            }
        ),
        "release": snapshot.get("release", {}) if snapshot else {},
        "requested_runs": snapshot.get("requested_runs") if snapshot else None,
        "subjects": valid_subjects,
        "totals": {
            "expected_reports": expected_reports,
            "received_reports": received_reports,
            "report_coverage": (
                received_reports / expected_reports * 100 if expected_reports else 0
            ),
            "passed_scenarios": passed_scenarios,
            "scenario_pass_rate": (
                passed_scenarios / expected_reports * 100 if expected_reports else 0
            ),
            "average_score": sum(scores) / len(scores) if scores else None,
            "total_cost_usd": sum_complete(subject_costs),
            "wall_time_seconds": sum_complete(subject_wall_times),
            "hard_gate_failures": hard_gate_failures,
            "technical_failures": technical_failures,
            "missing_reports": missing_reports,
            "retries": retries,
        },
        "execution": {**snapshot_execution, **metadata},
    }


def sort_key(execution: dict[str, Any]) -> tuple[str, int]:
    timestamp = (
        execution.get("completed_at")
        or execution.get("started_at")
        or execution.get("generated_at")
        or ""
    )
    return str(timestamp), int(execution.get("attempt") or 0)


def _metadata_from_summary(execution: dict[str, Any]) -> dict[str, Any]:
    source = execution.get("source", {})
    if not isinstance(source, dict):
        source = {}
    return {
        "id": str(execution.get("id") or ""),
        "run_id": str(execution.get("run_id") or ""),
        "attempt": int(optional_number(execution.get("attempt")) or 1),
        "workflow_name": str(execution.get("workflow_name") or ""),
        "workflow_url": str(execution.get("workflow_url") or ""),
        "event": str(execution.get("event") or ""),
        "actor": str(execution.get("actor") or ""),
        "started_at": str(execution.get("started_at") or ""),
        "completed_at": str(execution.get("completed_at") or ""),
        "conclusion": str(execution.get("conclusion") or ""),
        "head_sha": str(source.get("sha") or ""),
        "head_branch": str(source.get("ref") or ""),
        "repository": str(source.get("repository") or ""),
    }


def sanitize_retained_details(
    site_dir: Path,
    manifest: dict[str, Any],
) -> None:
    """Migrate retained schema 2 reports before the updated site is published."""
    runs_dir = site_dir / "runs"
    for execution in manifest.get("executions", []):
        if not isinstance(execution, dict):
            continue
        relative_path = execution.get("detail_path")
        if not isinstance(relative_path, str) or not relative_path.startswith("runs/"):
            continue
        candidate = site_dir / relative_path
        if candidate.parent != runs_dir or not candidate.is_file():
            continue
        retained = load_json(candidate)
        if retained is None:
            continue
        public_detail = compact_public_detail(
            retained,
            _metadata_from_summary(execution),
        )
        candidate.write_text(json.dumps(public_detail, indent=2, sort_keys=True) + "\n")


def publish(
    site_dir: Path,
    *,
    snapshot_path: Path | None,
    detail_path: Path | None,
    jobs_path: Path | None = None,
    metadata: dict[str, Any],
    repo_url: str,
    max_summaries: int,
    max_details: int,
    site_mode: str = "published",
) -> dict[str, Any]:
    if max_summaries < 1 or max_details < 0 or max_details > max_summaries:
        raise PublishError("retention must satisfy 0 <= details <= summaries")
    if site_mode not in {"local", "published"}:
        raise PublishError("site mode must be local or published")
    site_dir.mkdir(parents=True, exist_ok=True)
    runs_dir = site_dir / "runs"
    runs_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = site_dir / "executions.js"
    manifest = load_manifest(manifest_path)
    sanitize_retained_details(site_dir, manifest)
    raw_snapshot = load_json(snapshot_path)
    raw_detail = load_json(detail_path)
    snapshot, snapshot_identity_failure = validate_artifact_identity(
        raw_snapshot,
        metadata,
        label="benchmark snapshot",
    )
    detail, detail_identity_failure = validate_artifact_identity(
        raw_detail,
        metadata,
        label="execution detail",
    )
    summary_snapshot = snapshot or detail
    artifact_failure = (
        snapshot_identity_failure or detail_identity_failure
        if summary_snapshot is None
        else None
    )
    jobs_document = load_json(jobs_path)
    summary = build_summary(
        summary_snapshot,
        metadata,
        detail=detail,
        jobs_document=jobs_document,
        artifact_failure=artifact_failure,
    )

    existing_by_id = {
        str(entry.get("id")): entry
        for entry in manifest.get("executions", [])
        if isinstance(entry, dict) and entry.get("id")
    }
    previous = existing_by_id.get(metadata["id"])
    if (
        snapshot is None
        and previous
        and previous.get("availability") in {"aggregate", "full"}
    ):
        preserved = dict(previous)
        preserved.update(
            {
                key: summary[key]
                for key in (
                    "workflow_name",
                    "workflow_url",
                    "event",
                    "actor",
                    "started_at",
                    "completed_at",
                    "conclusion",
                    "workflow_duration_seconds",
                )
            }
        )
        summary = preserved

    if detail is not None:
        scenario_metrics = build_scenario_metrics(detail)
        efficiency_totals = build_execution_efficiency_totals(detail)
        public_detail = compact_public_detail(detail, metadata)
        relative_detail_path = f"runs/{metadata['id']}.json"
        (site_dir / relative_detail_path).write_text(
            json.dumps(public_detail, indent=2, sort_keys=True) + "\n"
        )
        summary["availability"] = "full"
        summary["detail_path"] = relative_detail_path
        summary["scenario_metrics"] = scenario_metrics
        summary["totals"].update(efficiency_totals)

    existing_by_id[metadata["id"]] = summary
    executions = sorted(existing_by_id.values(), key=sort_key, reverse=True)
    dropped = executions[max_summaries:]
    executions = executions[:max_summaries]

    for entry in dropped:
        stored_path = entry.get("detail_path")
        if isinstance(stored_path, str) and stored_path.startswith("runs/"):
            candidate = site_dir / stored_path
            if candidate.parent == runs_dir and candidate.is_file():
                candidate.unlink()

    for index, entry in enumerate(executions):
        if index < max_details:
            continue
        stored_path = entry.get("detail_path")
        if isinstance(stored_path, str) and stored_path.startswith("runs/"):
            candidate = site_dir / stored_path
            if candidate.parent == runs_dir and candidate.is_file():
                candidate.unlink()
        entry["detail_path"] = None
        entry["availability"] = (
            "aggregate" if entry.get("subjects") else "unavailable"
        )

    retained_paths = {
        str(entry.get("detail_path"))
        for entry in executions
        if isinstance(entry.get("detail_path"), str)
        and str(entry["detail_path"]).startswith("runs/")
    }
    for candidate in runs_dir.glob("*.json"):
        if f"runs/{candidate.name}" not in retained_paths:
            candidate.unlink()

    updated = {
        "schema_version": SCHEMA_VERSION,
        "mode": site_mode,
        "last_update": metadata["completed_at"] or metadata["started_at"],
        "repo_url": repo_url,
        "retention": {
            "summaries": max_summaries,
            "details": max_details,
        },
        "executions": executions,
    }
    manifest_path.write_text(
        MANIFEST_PREFIX
        + json.dumps(updated, indent=2, sort_keys=True, ensure_ascii=False)
        + ";\n"
    )
    return updated


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--site-dir", type=Path, required=True)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--detail", type=Path)
    parser.add_argument("--jobs", type=Path)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--attempt", type=int, required=True)
    parser.add_argument("--workflow-name", required=True)
    parser.add_argument("--workflow-url", required=True)
    parser.add_argument("--event", required=True)
    parser.add_argument("--actor", default="")
    parser.add_argument("--started-at", default="")
    parser.add_argument("--completed-at", default="")
    parser.add_argument("--conclusion", required=True)
    parser.add_argument("--head-sha", default="")
    parser.add_argument("--head-branch", default="")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--repo-url", required=True)
    parser.add_argument("--max-summaries", type=int, default=100)
    parser.add_argument("--max-details", type=int, default=30)
    parser.add_argument(
        "--site-mode",
        choices=("local", "published"),
        default="published",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.attempt < 1:
        raise PublishError("attempt must be positive")
    metadata = {
        "id": f"{args.run_id}-{args.attempt}",
        "run_id": args.run_id,
        "attempt": args.attempt,
        "workflow_name": args.workflow_name,
        "workflow_url": args.workflow_url,
        "event": args.event,
        "actor": args.actor,
        "started_at": args.started_at,
        "completed_at": args.completed_at,
        "conclusion": args.conclusion,
        "head_sha": args.head_sha,
        "head_branch": args.head_branch,
        "repository": args.repository,
    }
    updated = publish(
        args.site_dir,
        snapshot_path=args.snapshot,
        detail_path=args.detail,
        jobs_path=args.jobs,
        metadata=metadata,
        repo_url=args.repo_url,
        max_summaries=args.max_summaries,
        max_details=args.max_details,
        site_mode=args.site_mode,
    )
    print(
        json.dumps(
            {
                "execution_id": metadata["id"],
                "summaries": len(updated["executions"]),
                "full_details": sum(
                    entry.get("availability") == "full"
                    for entry in updated["executions"]
                ),
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
