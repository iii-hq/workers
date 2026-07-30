#!/usr/bin/env python3
"""Convert Harness E2E reports into compact benchmark and dashboard data."""

from __future__ import annotations

import argparse
import json
import math
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 2


class CollectionError(ValueError):
    """Raised when E2E benchmark inputs are malformed or contradictory."""


@dataclass(frozen=True)
class CollectionConfig:
    reports_root: Path
    output_dir: Path
    subjects: list[dict[str, str]]
    scenarios: list[str]
    lane: str
    requested_runs: int
    source_sha: str
    source_ref: str
    repository: str
    workflow_url: str
    release_tag: str
    release_worker: str
    release_version: str
    release_url: str
    registry_tag: str
    judge_model: str
    judge_provider: str
    execution_run_id: str
    execution_attempt: int
    execution_event: str
    execution_actor: str
    generated_at: str


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise CollectionError(f"cannot decode {path}: {exc}") from exc


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise CollectionError(f"{label} must be a non-empty string")
    return value


def require_number(value: Any, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise CollectionError(f"{label} must be a number")
    number = float(value)
    if not math.isfinite(number):
        raise CollectionError(f"{label} must be finite")
    return number


def optional_number(value: Any, label: str) -> float | None:
    if value is None:
        return None
    return require_number(value, label)


def parse_subjects(raw: str) -> list[dict[str, str]]:
    try:
        subjects = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise CollectionError(f"subjects JSON is invalid: {exc}") from exc
    if not isinstance(subjects, list) or not subjects:
        raise CollectionError("subjects must be a non-empty JSON array")

    parsed: list[dict[str, str]] = []
    seen: set[str] = set()
    for index, subject in enumerate(subjects):
        if not isinstance(subject, dict):
            raise CollectionError(f"subjects[{index}] must be an object")
        entry = {
            "id": require_string(subject.get("id"), f"subjects[{index}].id"),
            "model": require_string(
                subject.get("model"), f"subjects[{index}].model"
            ),
            "provider": require_string(
                subject.get("provider"), f"subjects[{index}].provider"
            ),
        }
        if entry["id"] in seen:
            raise CollectionError(f"duplicate subject id: {entry['id']}")
        seen.add(entry["id"])
        parsed.append(entry)
    return parsed


def parse_scenarios(raw: str) -> list[str]:
    try:
        scenarios = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise CollectionError(f"scenarios JSON is invalid: {exc}") from exc
    if not isinstance(scenarios, list) or not scenarios:
        raise CollectionError("scenarios must be a non-empty JSON array")
    parsed = [require_string(value, "scenario") for value in scenarios]
    if len(set(parsed)) != len(parsed):
        raise CollectionError("scenarios must be unique")
    return parsed


def discover_contexts(root: Path) -> dict[tuple[str, str], Path]:
    contexts: dict[tuple[str, str], Path] = {}
    if not root.exists():
        return contexts
    for path in sorted(root.rglob("benchmark-context.json")):
        context = load_json(path)
        if not isinstance(context, dict):
            raise CollectionError(f"{path} must contain an object")
        subject_id = require_string(context.get("subject_id"), f"{path}: subject_id")
        scenario_id = require_string(
            context.get("scenario_id"), f"{path}: scenario_id"
        )
        key = (subject_id, scenario_id)
        if key in contexts:
            raise CollectionError(
                f"duplicate benchmark context for {subject_id}/{scenario_id}"
            )
        contexts[key] = path
    return contexts


def compact_extra(value: dict[str, Any]) -> str:
    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def metric(
    category: str,
    subject_id: str,
    scenario_id: str,
    metric_id: str,
    unit: str,
    value: float,
    extra: dict[str, Any],
    *,
    value_range: str | None = None,
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "name": f"{category}::{subject_id}::{scenario_id}::{metric_id}",
        "unit": unit,
        "value": value,
        "extra": compact_extra(extra),
    }
    if value_range:
        result["range"] = value_range
    return result


def sum_known(values: list[float | None], *, require_all: bool = True) -> float | None:
    if require_all and any(value is None for value in values):
        return None
    known = [value for value in values if value is not None]
    return sum(known) if known else None


def validate_report(
    report: Any,
    *,
    subject: dict[str, str],
    scenario_id: str,
    path: Path,
) -> tuple[dict[str, Any], dict[str, Any]]:
    if not isinstance(report, dict):
        raise CollectionError(f"{path} must contain an object")
    report_subject = report.get("subject")
    if not isinstance(report_subject, dict):
        raise CollectionError(f"{path}: subject must be an object")
    if (
        report_subject.get("model") != subject["model"]
        or report_subject.get("provider") != subject["provider"]
    ):
        raise CollectionError(
            f"{path}: report subject does not match {subject['id']}"
        )
    scenarios = report.get("scenarios")
    if not isinstance(scenarios, list) or len(scenarios) != 1:
        raise CollectionError(f"{path}: expected exactly one scenario")
    scenario = scenarios[0]
    if not isinstance(scenario, dict) or scenario.get("scenario_id") != scenario_id:
        raise CollectionError(f"{path}: scenario id does not match {scenario_id}")
    return report, scenario


def collect(
    config: CollectionConfig,
) -> tuple[
    list[dict[str, Any]],
    list[dict[str, Any]],
    dict[str, Any],
    dict[str, Any],
]:
    contexts = discover_contexts(config.reports_root)
    quality: list[dict[str, Any]] = []
    efficiency: list[dict[str, Any]] = []
    snapshot_subjects: list[dict[str, Any]] = []
    execution_reports: list[dict[str, Any]] = []
    execution_id = f"{config.execution_run_id}-{config.execution_attempt}"
    execution = {
        "id": execution_id,
        "run_id": config.execution_run_id,
        "attempt": config.execution_attempt,
        "event": config.execution_event,
        "actor": config.execution_actor,
        "workflow_url": config.workflow_url,
    }

    for subject in config.subjects:
        scenario_snapshots: list[dict[str, Any]] = []
        subject_costs: list[float | None] = []
        subject_wall_times: list[float | None] = []
        subject_passed = 0
        report_count = 0
        hard_gate_failures = 0
        technical_failures = 0
        retries = 0
        engine_revisions: set[str] = set()
        resolved_judge: dict[str, Any] | None = None

        for scenario_id in config.scenarios:
            context_path = contexts.get((subject["id"], scenario_id))
            report_path = (
                context_path.parent / "results.json" if context_path is not None else None
            )
            base_extra: dict[str, Any] = {
                "schema_version": SCHEMA_VERSION,
                "execution": execution,
                "lane": config.lane,
                "generated_at": config.generated_at,
                "source": {
                    "sha": config.source_sha,
                    "ref": config.source_ref,
                    "repository": config.repository,
                },
                "workflow_url": config.workflow_url,
                "release": {
                    "tag": config.release_tag,
                    "worker": config.release_worker,
                    "version": config.release_version,
                    "url": config.release_url,
                    "registry_tag": config.registry_tag,
                },
                "subject": subject,
                "judge": {
                    "model": config.judge_model,
                    "provider": config.judge_provider,
                },
                "scenario": scenario_id,
                "requested_runs": config.requested_runs,
            }

            if report_path is None or not report_path.is_file():
                execution_reports.append(
                    {
                        "subject_id": subject["id"],
                        "scenario_id": scenario_id,
                        "available": False,
                        "report": None,
                    }
                )
                scenario_snapshot = {
                    "id": scenario_id,
                    "status": "missing_report",
                    "passed": False,
                    "threshold": None,
                    "runs": 0,
                    "median_score": None,
                    "pass_rate": None,
                    "hard_gate_failures": None,
                    "technical_failures": None,
                    "retries": None,
                    "total_cost_usd": None,
                    "wall_time_seconds": None,
                }
                efficiency.append(
                    metric(
                        "reliability",
                        subject["id"],
                        scenario_id,
                        "missing_reports",
                        "count",
                        1,
                        {**base_extra, "passed": False, "status": "missing_report"},
                    )
                )
                subject_costs.append(None)
                subject_wall_times.append(None)
                scenario_snapshots.append(scenario_snapshot)
                continue

            report, scenario = validate_report(
                load_json(report_path),
                subject=subject,
                scenario_id=scenario_id,
                path=report_path,
            )
            execution_reports.append(
                {
                    "subject_id": subject["id"],
                    "scenario_id": scenario_id,
                    "available": True,
                    "report": report,
                }
            )
            report_count += 1
            report_passed = bool(scenario.get("passed"))
            subject_passed += int(report_passed)
            aggregate = scenario.get("aggregate")
            runs = scenario.get("runs")
            if not isinstance(aggregate, dict) or not isinstance(runs, list):
                raise CollectionError(f"{report_path}: aggregate and runs are required")

            threshold = require_number(
                scenario.get("threshold"), f"{report_path}: threshold"
            )
            median_score = optional_number(
                aggregate.get("median_score"), f"{report_path}: median_score"
            )
            pass_rate = require_number(
                aggregate.get("pass_rate"), f"{report_path}: pass_rate"
            )
            hard_gates = int(
                require_number(
                    aggregate.get("hard_gate_failures"),
                    f"{report_path}: hard_gate_failures",
                )
            )
            technical = int(
                require_number(
                    aggregate.get("technical_failures"),
                    f"{report_path}: technical_failures",
                )
            )
            retry_count = sum(
                len(run.get("retry_attempts", []))
                for run in runs
                if isinstance(run, dict)
                and isinstance(run.get("retry_attempts", []), list)
            )
            wall_time_seconds = (
                sum(
                    require_number(
                        run.get("wall_time_ms"), f"{report_path}: run wall_time_ms"
                    )
                    for run in runs
                    if isinstance(run, dict)
                )
                / 1000
            )
            aggregate_cost = aggregate.get("cost")
            if not isinstance(aggregate_cost, dict):
                raise CollectionError(f"{report_path}: aggregate.cost is required")
            total_cost = optional_number(
                aggregate_cost.get("total_usd"), f"{report_path}: total cost"
            )
            subject_cost = optional_number(
                aggregate_cost.get("subject_usd"), f"{report_path}: subject cost"
            )
            judge_cost = optional_number(
                aggregate_cost.get("judge_usd"), f"{report_path}: judge cost"
            )
            score_values = [
                int(require_number(run["score"], f"{report_path}: run score"))
                for run in runs
                if isinstance(run, dict) and run.get("score") is not None
            ]

            engine_revision = report.get("engine_revision")
            if isinstance(engine_revision, str) and engine_revision:
                engine_revisions.add(engine_revision)
            if isinstance(report.get("judge"), dict):
                resolved_judge = report["judge"]

            status = "passed" if report_passed else "failed"
            extra = {
                **base_extra,
                "judge": report.get("judge") or base_extra["judge"],
                "engine_revision": engine_revision,
                "threshold": threshold,
                "passed": report_passed,
                "status": status,
                "runs": len(runs),
            }

            if median_score is not None:
                score_range = (
                    f"{min(score_values)}–{max(score_values)}"
                    if len(score_values) > 1
                    else None
                )
                quality.append(
                    metric(
                        "quality",
                        subject["id"],
                        scenario_id,
                        "median_score",
                        "points",
                        median_score,
                        extra,
                        value_range=score_range,
                    )
                )
            quality.append(
                metric(
                    "quality",
                    subject["id"],
                    scenario_id,
                    "pass_rate",
                    "percent",
                    pass_rate * 100,
                    extra,
                )
            )
            efficiency.extend(
                [
                    metric(
                        "reliability",
                        subject["id"],
                        scenario_id,
                        "hard_gate_failures",
                        "count",
                        hard_gates,
                        extra,
                    ),
                    metric(
                        "reliability",
                        subject["id"],
                        scenario_id,
                        "technical_failures",
                        "count",
                        technical,
                        extra,
                    ),
                    metric(
                        "reliability",
                        subject["id"],
                        scenario_id,
                        "retry_attempts",
                        "count",
                        retry_count,
                        extra,
                    ),
                    metric(
                        "reliability",
                        subject["id"],
                        scenario_id,
                        "missing_reports",
                        "count",
                        0,
                        extra,
                    ),
                    metric(
                        "efficiency",
                        subject["id"],
                        scenario_id,
                        "wall_time_seconds",
                        "seconds",
                        wall_time_seconds,
                        extra,
                    ),
                ]
            )
            if subject_cost is not None:
                efficiency.append(
                    metric(
                        "efficiency",
                        subject["id"],
                        scenario_id,
                        "subject_cost_usd",
                        "USD",
                        subject_cost,
                        extra,
                    )
                )
            if judge_cost is not None:
                efficiency.append(
                    metric(
                        "efficiency",
                        subject["id"],
                        scenario_id,
                        "judge_cost_usd",
                        "USD",
                        judge_cost,
                        extra,
                    )
                )
            if total_cost is not None:
                efficiency.append(
                    metric(
                        "efficiency",
                        subject["id"],
                        scenario_id,
                        "total_cost_usd",
                        "USD",
                        total_cost,
                        extra,
                    )
                )

            hard_gate_failures += hard_gates
            technical_failures += technical
            retries += retry_count
            subject_costs.append(total_cost)
            subject_wall_times.append(wall_time_seconds)
            scenario_snapshots.append(
                {
                    "id": scenario_id,
                    "status": status,
                    "passed": report_passed,
                    "threshold": threshold,
                    "runs": len(runs),
                    "median_score": median_score,
                    "pass_rate": pass_rate,
                    "hard_gate_failures": hard_gates,
                    "technical_failures": technical,
                    "retries": retry_count,
                    "total_cost_usd": total_cost,
                    "wall_time_seconds": wall_time_seconds,
                }
            )

        expected_count = len(config.scenarios)
        missing_reports = expected_count - report_count
        scenario_pass_rate = subject_passed / expected_count * 100
        report_coverage = report_count / expected_count * 100
        all_reports_present = missing_reports == 0
        total_cost = sum_known(subject_costs)
        total_wall_time = sum_known(subject_wall_times)
        suite_passed = (
            all_reports_present
            and subject_passed == expected_count
            and hard_gate_failures == 0
            and technical_failures == 0
        )
        engine_revision = (
            next(iter(engine_revisions)) if len(engine_revisions) == 1 else None
        )
        suite_extra = {
            "schema_version": SCHEMA_VERSION,
            "execution": execution,
            "lane": config.lane,
            "generated_at": config.generated_at,
            "source": {
                "sha": config.source_sha,
                "ref": config.source_ref,
                "repository": config.repository,
            },
            "workflow_url": config.workflow_url,
            "release": {
                "tag": config.release_tag,
                "worker": config.release_worker,
                "version": config.release_version,
                "url": config.release_url,
                "registry_tag": config.registry_tag,
            },
            "subject": subject,
            "judge": resolved_judge
            or {"model": config.judge_model, "provider": config.judge_provider},
            "engine_revision": engine_revision,
            "scenario": "suite",
            "requested_runs": config.requested_runs,
            "passed": suite_passed,
            "status": "passed" if suite_passed else "failed",
            "expected_reports": expected_count,
            "received_reports": report_count,
        }
        quality.extend(
            [
                metric(
                    "quality",
                    subject["id"],
                    "suite",
                    "scenario_pass_rate",
                    "percent",
                    scenario_pass_rate,
                    suite_extra,
                ),
                metric(
                    "quality",
                    subject["id"],
                    "suite",
                    "report_coverage",
                    "percent",
                    report_coverage,
                    suite_extra,
                ),
            ]
        )
        efficiency.extend(
            [
                metric(
                    "reliability",
                    subject["id"],
                    "suite",
                    "hard_gate_failures",
                    "count",
                    hard_gate_failures,
                    suite_extra,
                ),
                metric(
                    "reliability",
                    subject["id"],
                    "suite",
                    "technical_failures",
                    "count",
                    technical_failures,
                    suite_extra,
                ),
                metric(
                    "reliability",
                    subject["id"],
                    "suite",
                    "retry_attempts",
                    "count",
                    retries,
                    suite_extra,
                ),
                metric(
                    "reliability",
                    subject["id"],
                    "suite",
                    "missing_reports",
                    "count",
                    missing_reports,
                    suite_extra,
                ),
            ]
        )
        if total_cost is not None:
            efficiency.append(
                metric(
                    "efficiency",
                    subject["id"],
                    "suite",
                    "total_cost_usd",
                    "USD",
                    total_cost,
                    suite_extra,
                )
            )
        if total_wall_time is not None:
            efficiency.append(
                metric(
                    "efficiency",
                    subject["id"],
                    "suite",
                    "wall_time_seconds",
                    "seconds",
                    total_wall_time,
                    suite_extra,
                )
            )

        snapshot_subjects.append(
            {
                **subject,
                "judge": suite_extra["judge"],
                "engine_revision": engine_revision,
                "passed": suite_passed,
                "expected_reports": expected_count,
                "received_reports": report_count,
                "scenario_pass_rate": scenario_pass_rate / 100,
                "report_coverage": report_coverage / 100,
                "hard_gate_failures": hard_gate_failures,
                "technical_failures": technical_failures,
                "retry_attempts": retries,
                "total_cost_usd": total_cost,
                "wall_time_seconds": total_wall_time,
                "scenarios": scenario_snapshots,
            }
        )

    snapshot = {
        "schema_version": SCHEMA_VERSION,
        "execution": execution,
        "generated_at": config.generated_at,
        "lane": config.lane,
        "source": {
            "sha": config.source_sha,
            "ref": config.source_ref,
            "repository": config.repository,
        },
        "workflow_url": config.workflow_url,
        "release": {
            "tag": config.release_tag,
            "worker": config.release_worker,
            "version": config.release_version,
            "url": config.release_url,
            "registry_tag": config.registry_tag,
        },
        "requested_runs": config.requested_runs,
        "subjects": snapshot_subjects,
    }
    execution_detail = {
        **snapshot,
        "reports": execution_reports,
    }
    return quality, efficiency, snapshot, execution_detail


def write_outputs(
    output_dir: Path,
    quality: list[dict[str, Any]],
    efficiency: list[dict[str, Any]],
    snapshot: dict[str, Any],
    execution: dict[str, Any],
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    for name, payload in (
        ("quality.json", quality),
        ("efficiency.json", efficiency),
        ("snapshot.json", snapshot),
        ("execution.json", execution),
    ):
        (output_dir / name).write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n"
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reports-root", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--subjects-json", required=True)
    parser.add_argument("--scenarios-json", required=True)
    parser.add_argument("--lane", required=True)
    parser.add_argument("--requested-runs", type=int, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--source-ref", default="")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--workflow-url", required=True)
    parser.add_argument("--release-tag", default="")
    parser.add_argument("--release-worker", default="")
    parser.add_argument("--release-version", default="")
    parser.add_argument("--release-url", default="")
    parser.add_argument("--registry-tag", default="")
    parser.add_argument("--judge-model", required=True)
    parser.add_argument("--judge-provider", required=True)
    parser.add_argument("--execution-run-id", required=True)
    parser.add_argument("--execution-attempt", type=int, required=True)
    parser.add_argument("--execution-event", default="")
    parser.add_argument("--execution-actor", default="")
    parser.add_argument("--generated-at")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.requested_runs < 1:
        raise CollectionError("requested runs must be positive")
    if args.execution_attempt < 1:
        raise CollectionError("execution attempt must be positive")
    generated_at = args.generated_at or datetime.now(timezone.utc).isoformat()
    config = CollectionConfig(
        reports_root=args.reports_root,
        output_dir=args.output_dir,
        subjects=parse_subjects(args.subjects_json),
        scenarios=parse_scenarios(args.scenarios_json),
        lane=require_string(args.lane, "lane"),
        requested_runs=args.requested_runs,
        source_sha=require_string(args.source_sha, "source SHA"),
        source_ref=args.source_ref,
        repository=require_string(args.repository, "repository"),
        workflow_url=require_string(args.workflow_url, "workflow URL"),
        release_tag=args.release_tag,
        release_worker=args.release_worker,
        release_version=args.release_version,
        release_url=args.release_url,
        registry_tag=args.registry_tag,
        judge_model=require_string(args.judge_model, "judge model"),
        judge_provider=require_string(args.judge_provider, "judge provider"),
        execution_run_id=require_string(args.execution_run_id, "execution run id"),
        execution_attempt=args.execution_attempt,
        execution_event=args.execution_event,
        execution_actor=args.execution_actor,
        generated_at=generated_at,
    )
    quality, efficiency, snapshot, execution = collect(config)
    write_outputs(args.output_dir, quality, efficiency, snapshot, execution)
    print(
        json.dumps(
            {
                "quality_metrics": len(quality),
                "efficiency_metrics": len(efficiency),
                "subjects": len(snapshot["subjects"]),
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
