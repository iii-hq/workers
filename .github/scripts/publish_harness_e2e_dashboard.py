#!/usr/bin/env python3
"""Update the static Harness E2E execution manifest and retained reports."""

from __future__ import annotations

import argparse
import json
from datetime import datetime
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 2
MANIFEST_PREFIX = "window.HARNESS_EXECUTIONS = "


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
    grouped_runs: dict[str, list[dict[str, Any]]] = {}
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
            runs = scenario.get("runs", [])
            if not isinstance(runs, list):
                continue
            grouped_runs.setdefault(scenario_id, []).extend(
                run for run in runs if isinstance(run, dict)
            )

    result = []
    for scenario_id, runs in sorted(grouped_runs.items()):
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
                "scenario_id": scenario_id,
                "run_count": len(runs),
                "averages": averages,
                "samples": samples,
            }
        )
    return result


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
) -> str:
    if conclusion == "cancelled":
        return "cancelled"
    if not subjects or expected_reports == 0 or received_reports < expected_reports:
        return "incomplete"
    if conclusion != "success" or not all(
        bool(subject.get("passed")) for subject in subjects
    ):
        return "failed"
    return "passed"


def build_summary(
    snapshot: dict[str, Any] | None,
    metadata: dict[str, Any],
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
    status = execution_status(
        conclusion,
        valid_subjects,
        expected_reports,
        received_reports,
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


def publish(
    site_dir: Path,
    *,
    snapshot_path: Path | None,
    detail_path: Path | None,
    metadata: dict[str, Any],
    repo_url: str,
    max_summaries: int,
    max_details: int,
) -> dict[str, Any]:
    if max_summaries < 1 or max_details < 0 or max_details > max_summaries:
        raise PublishError("retention must satisfy 0 <= details <= summaries")
    site_dir.mkdir(parents=True, exist_ok=True)
    runs_dir = site_dir / "runs"
    runs_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = site_dir / "executions.js"
    manifest = load_manifest(manifest_path)
    snapshot = load_json(snapshot_path)
    detail = load_json(detail_path)
    summary = build_summary(snapshot, metadata)

    existing_by_id = {
        str(entry.get("id")): entry
        for entry in manifest.get("executions", [])
        if isinstance(entry, dict) and entry.get("id")
    }
    previous = existing_by_id.get(metadata["id"])
    if snapshot is None and previous:
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
        detail["schema_version"] = SCHEMA_VERSION
        execution_metadata = detail.get("execution", {})
        if not isinstance(execution_metadata, dict):
            execution_metadata = {}
        detail["execution"] = {**execution_metadata, **metadata}
        relative_detail_path = f"runs/{metadata['id']}.json"
        (site_dir / relative_detail_path).write_text(
            json.dumps(detail, indent=2, sort_keys=True) + "\n"
        )
        summary["availability"] = "full"
        summary["detail_path"] = relative_detail_path
        summary["scenario_metrics"] = build_scenario_metrics(detail)

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

    updated = {
        "schema_version": SCHEMA_VERSION,
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
        metadata=metadata,
        repo_url=args.repo_url,
        max_summaries=args.max_summaries,
        max_details=args.max_details,
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
