#!/usr/bin/env python3
"""Import local Harness E2E reports and serve the benchmark dashboard."""

from __future__ import annotations

import argparse
import functools
import getpass
import hashlib
import http.server
import json
import re
import shutil
import subprocess
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from collect_harness_e2e_benchmarks import (
    CollectionConfig,
    CollectionError,
    collect,
    write_outputs,
)
from publish_harness_e2e_dashboard import (
    MANIFEST_PREFIX,
    PublishError,
    load_manifest,
    publish,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
DASHBOARD_SOURCE = REPO_ROOT / ".github" / "benchmark-site"
DEFAULT_RESULTS = REPO_ROOT / "harness" / "target" / "e2e" / "results.json"
DEFAULT_SITE_DIR = REPO_ROOT / "target" / "harness-e2e-dashboard-local"
LOCAL_SITE_MARKER = ".harness-e2e-local-dashboard"
BENCHMARK_DATA_PREFIX = "window.BENCHMARK_DATA = "


class LocalDashboardError(ValueError):
    """Raised when local reports cannot be rendered safely."""


def load_report(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise LocalDashboardError(f"cannot decode {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise LocalDashboardError(f"{path} must contain a JSON object")
    subject = value.get("subject")
    if not isinstance(subject, dict):
        raise LocalDashboardError(f"{path}: subject must be an object")
    for field in ("model", "provider"):
        if not isinstance(subject.get(field), str) or not subject[field]:
            raise LocalDashboardError(f"{path}: subject.{field} is required")
    judge = value.get("judge")
    if judge is not None:
        if not isinstance(judge, dict):
            raise LocalDashboardError(f"{path}: judge must be an object or null")
        for field in ("model", "provider"):
            if not isinstance(judge.get(field), str) or not judge[field]:
                raise LocalDashboardError(f"{path}: judge.{field} is required")
    scenarios = value.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        raise LocalDashboardError(f"{path}: scenarios must be a non-empty array")
    scenario_ids = [
        scenario.get("scenario_id") if isinstance(scenario, dict) else None
        for scenario in scenarios
    ]
    if any(
        not isinstance(scenario_id, str) or not scenario_id
        for scenario_id in scenario_ids
    ):
        raise LocalDashboardError(f"{path}: every scenario must have a scenario_id")
    if len(set(scenario_ids)) != len(scenario_ids):
        raise LocalDashboardError(f"{path}: scenario ids must be unique")
    if not isinstance(value.get("passed"), bool):
        value["passed"] = all(
            bool(scenario.get("passed"))
            for scenario in scenarios
            if isinstance(scenario, dict)
        )
    return value


def discover_results(
    inputs: list[Path],
    *,
    site_dir: Path,
) -> list[Path]:
    candidates = inputs or [DEFAULT_RESULTS]
    site_root = site_dir.resolve()
    discovered: list[Path] = []
    seen: set[Path] = set()

    for candidate in candidates:
        path = candidate.expanduser().resolve()
        if path.is_file():
            matches = [path]
        elif path.is_dir():
            direct = path / "results.json"
            matches = (
                [direct] if direct.is_file() else sorted(path.rglob("results.json"))
            )
        else:
            raise LocalDashboardError(f"results path does not exist: {path}")

        matches = [
            match.resolve()
            for match in matches
            if not match.resolve().is_relative_to(site_root)
        ]
        if not matches:
            raise LocalDashboardError(f"no results.json found under {path}")
        for match in matches:
            if match not in seen:
                seen.add(match)
                discovered.append(match)

    return sorted(discovered, key=lambda path: (path.stat().st_mtime_ns, str(path)))


def report_digest(report: dict[str, Any]) -> str:
    canonical = json.dumps(
        report,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode()
    return hashlib.sha256(canonical).hexdigest()


def slug(value: str) -> str:
    normalized = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return normalized[:80] or "subject"


def git_value(*arguments: str) -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(REPO_ROOT), *arguments],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return ""
    return result.stdout.strip()


def repository_url() -> str:
    remote = git_value("remote", "get-url", "origin")
    ssh_match = re.fullmatch(r"git@github\.com:(.+?)(?:\.git)?", remote)
    if ssh_match:
        return f"https://github.com/{ssh_match.group(1)}"
    if remote.startswith(("https://", "http://")):
        return remote.removesuffix(".git")
    return "https://github.com/iii-hq/workers"


def repository_name(repo_url: str) -> str:
    match = re.search(r"github\.com/([^/]+/[^/]+?)(?:\.git)?$", repo_url)
    return match.group(1) if match else "iii-hq/workers"


def report_wall_time(report: dict[str, Any]) -> float:
    total_ms = 0.0
    for scenario in report["scenarios"]:
        runs = scenario.get("runs", [])
        if not isinstance(runs, list):
            continue
        for run in runs:
            wall_time_ms = run.get("wall_time_ms") if isinstance(run, dict) else None
            if isinstance(wall_time_ms, (int, float)) and not isinstance(
                wall_time_ms, bool
            ):
                total_ms += max(0.0, float(wall_time_ms))
    return total_ms / 1000


def requested_runs(report: dict[str, Any]) -> int:
    values = []
    for scenario in report["scenarios"]:
        aggregate = scenario.get("aggregate", {})
        aggregate_runs = (
            aggregate.get("runs") if isinstance(aggregate, dict) else None
        )
        runs = scenario.get("runs", [])
        if (
            isinstance(aggregate_runs, int)
            and not isinstance(aggregate_runs, bool)
            and aggregate_runs > 0
        ):
            values.append(aggregate_runs)
        elif isinstance(runs, list) and runs:
            values.append(len(runs))
    return max(values, default=1)


def prepare_site(site_dir: Path, *, reset: bool) -> None:
    if reset and site_dir.exists():
        marker = site_dir / LOCAL_SITE_MARKER
        if not marker.is_file():
            raise LocalDashboardError(
                f"refusing to reset unmarked directory: {site_dir}"
            )
        shutil.rmtree(site_dir)
    if site_dir.exists() and not site_dir.is_dir():
        raise LocalDashboardError(f"site path is not a directory: {site_dir}")
    site_dir.mkdir(parents=True, exist_ok=True)
    shutil.copytree(DASHBOARD_SOURCE, site_dir, dirs_exist_ok=True)
    (site_dir / LOCAL_SITE_MARKER).write_text("Harness E2E local dashboard\n")


def write_benchmark_data(site_dir: Path, repo_url: str, last_update: str) -> None:
    timestamp = 0
    if last_update:
        try:
            timestamp = int(datetime.fromisoformat(last_update).timestamp() * 1000)
        except ValueError:
            timestamp = 0
    payload = {
        "entries": {},
        "lastUpdate": timestamp,
        "repoUrl": repo_url,
    }
    (site_dir / "data.js").write_text(
        BENCHMARK_DATA_PREFIX
        + json.dumps(payload, indent=2, sort_keys=True)
        + ";\n"
    )


def stage_report(
    report: dict[str, Any],
    reports_root: Path,
    subject_id: str,
) -> list[str]:
    scenario_ids = []
    for scenario in report["scenarios"]:
        scenario_id = scenario["scenario_id"]
        scenario_ids.append(scenario_id)
        directory = reports_root / f"{subject_id}-{slug(scenario_id)}"
        directory.mkdir(parents=True)
        (directory / "benchmark-context.json").write_text(
            json.dumps(
                {"subject_id": subject_id, "scenario_id": scenario_id},
                sort_keys=True,
            )
            + "\n"
        )
        scenario_report = {
            **report,
            "passed": bool(scenario.get("passed")),
            "scenarios": [scenario],
        }
        (directory / "results.json").write_text(
            json.dumps(scenario_report, indent=2, sort_keys=True) + "\n"
        )
    return scenario_ids


def import_report(
    results_path: Path,
    *,
    site_dir: Path,
    repo_url: str,
    repo_name: str,
    source_sha: str,
    source_ref: str,
) -> str:
    report = load_report(results_path)
    digest = report_digest(report)
    run_id = f"local-{digest[:12]}"
    subject = report["subject"]
    subject_id = slug(f"{subject['provider']}-{subject['model']}")
    judge = report.get("judge")
    if not isinstance(judge, dict):
        judge = subject

    completed = datetime.fromtimestamp(
        results_path.stat().st_mtime,
        timezone.utc,
    )
    started = completed - timedelta(seconds=report_wall_time(report))
    completed_at = completed.isoformat()
    started_at = started.isoformat()

    with tempfile.TemporaryDirectory(prefix="harness-e2e-local-dashboard-") as temp:
        temp_root = Path(temp)
        reports_root = temp_root / "reports"
        scenarios = stage_report(report, reports_root, subject_id)
        output_dir = temp_root / "output"
        config = CollectionConfig(
            reports_root=reports_root,
            output_dir=output_dir,
            subjects=[
                {
                    "id": subject_id,
                    "model": subject["model"],
                    "provider": subject["provider"],
                }
            ],
            scenarios=scenarios,
            lane="local",
            requested_runs=requested_runs(report),
            source_sha=source_sha,
            source_ref=source_ref,
            repository=repo_name,
            workflow_url="",
            release_tag="",
            release_worker="",
            release_version="",
            release_url="",
            registry_tag="local",
            judge_model=str(judge["model"]),
            judge_provider=str(judge["provider"]),
            execution_run_id=run_id,
            execution_attempt=1,
            execution_event="local",
            execution_actor=getpass.getuser(),
            generated_at=completed_at,
        )
        quality, efficiency, snapshot, execution = collect(config)
        if report.get("judge") is None:
            snapshot["subjects"][0]["judge"] = {}
        has_blocking_failure = any(
            int(subject.get("hard_gate_failures") or 0) > 0
            or int(subject.get("technical_failures") or 0) > 0
            for subject in snapshot["subjects"]
            if isinstance(subject, dict)
        )
        write_outputs(output_dir, quality, efficiency, snapshot, execution)
        publish(
            site_dir,
            snapshot_path=output_dir / "snapshot.json",
            detail_path=output_dir / "execution.json",
            metadata={
                "id": f"{run_id}-1",
                "run_id": run_id,
                "attempt": 1,
                "workflow_name": "Harness E2E Local",
                "workflow_url": "",
                "event": "local",
                "actor": getpass.getuser(),
                "started_at": started_at,
                "completed_at": completed_at,
                # Quality is advisory in CI and must remain advisory in a local
                # import. Only technical and hard-gate outcomes fail the run.
                "conclusion": "failure" if has_blocking_failure else "success",
                "head_sha": source_sha,
                "head_branch": source_ref,
                "repository": repo_name,
            },
            repo_url=repo_url,
            max_summaries=100,
            max_details=30,
            site_mode="local",
        )
    return f"{run_id}-1"


def build_local_dashboard(
    results_paths: list[Path],
    *,
    site_dir: Path,
    reset: bool = False,
) -> list[str]:
    site_dir = site_dir.expanduser().resolve()
    paths = discover_results(results_paths, site_dir=site_dir)
    prepare_site(site_dir, reset=reset)
    repo_url = repository_url()
    repo_name = repository_name(repo_url)
    source_sha = git_value("rev-parse", "HEAD") or "local"
    source_ref = git_value("branch", "--show-current") or "local"
    execution_ids = [
        import_report(
            path,
            site_dir=site_dir,
            repo_url=repo_url,
            repo_name=repo_name,
            source_sha=source_sha,
            source_ref=source_ref,
        )
        for path in paths
    ]
    imported_at = datetime.now(timezone.utc).isoformat()
    manifest_path = site_dir / "executions.js"
    manifest = load_manifest(manifest_path)
    manifest["mode"] = "local"
    manifest["last_update"] = imported_at
    manifest_path.write_text(
        MANIFEST_PREFIX
        + json.dumps(manifest, indent=2, sort_keys=True, ensure_ascii=False)
        + ";\n"
    )
    write_benchmark_data(site_dir, repo_url, imported_at)
    return execution_ids


def serve(site_dir: Path, host: str, port: int) -> None:
    handler = functools.partial(
        http.server.SimpleHTTPRequestHandler,
        directory=str(site_dir),
    )
    try:
        server = http.server.ThreadingHTTPServer((host, port), handler)
    except OSError as exc:
        raise LocalDashboardError(
            f"cannot serve dashboard on {host}:{port}: {exc}"
        ) from exc
    visible_host = "127.0.0.1" if host in {"0.0.0.0", "::"} else host
    print(f"dashboard: http://{visible_host}:{server.server_port}/index.html")
    print("press Ctrl+C to stop")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nstopping dashboard")
    finally:
        server.server_close()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Import local Harness E2E results and serve the dashboard."
    )
    parser.add_argument(
        "results",
        nargs="*",
        type=Path,
        help="results.json files or directories containing local reports",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=4173)
    parser.add_argument("--site-dir", type=Path, default=DEFAULT_SITE_DIR)
    parser.add_argument(
        "--reset",
        action="store_true",
        help="clear previously imported local executions before importing",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not 1 <= args.port <= 65535:
        parser.error("--port must be between 1 and 65535")
    try:
        execution_ids = build_local_dashboard(
            args.results,
            site_dir=args.site_dir,
            reset=args.reset,
        )
        print(
            f"imported {len(execution_ids)} execution"
            f"{'' if len(execution_ids) == 1 else 's'}"
        )
        serve(args.site_dir.expanduser().resolve(), args.host, args.port)
    except (CollectionError, LocalDashboardError, PublishError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
