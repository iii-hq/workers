#!/usr/bin/env python3
"""Import local Harness E2E reports and serve the benchmark dashboard."""

from __future__ import annotations

import argparse
import functools
import getpass
import hashlib
import http.server
import ipaddress
import json
import os
import re
import signal
import shutil
import subprocess
import tempfile
import threading
import time
import uuid
from collections.abc import Mapping
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse, urlsplit

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
DEFAULT_RUNS_DIR = REPO_ROOT / "target" / "harness-e2e-local-runs"
DEFAULT_RUNNER_DEBUG = REPO_ROOT / "harness" / "target" / "debug" / "harness-e2e"
DEFAULT_RUNNER_RELEASE = REPO_ROOT / "harness" / "target" / "release" / "harness-e2e"
LOCAL_SITE_MARKER = ".harness-e2e-local-dashboard"
BENCHMARK_DATA_PREFIX = "window.BENCHMARK_DATA = "
MAX_REQUEST_BYTES = 64 * 1024
MAX_LOG_TAIL_BYTES = 32 * 1024
CATALOG_CACHE_SECONDS = 30
CATALOG_TIMEOUT_SECONDS = 120
SCENARIO_ID = re.compile(r"^[a-z0-9][a-z0-9_]*$")


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
    label: str = "",
    run_id: str | None = None,
) -> str:
    report = load_report(results_path)
    digest = report_digest(report)
    run_id = run_id or f"local-{digest[:12]}"
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
                "label": label,
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
    labels_by_path: dict[Path, str] | None = None,
    run_ids_by_path: dict[Path, str] | None = None,
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
            label=(labels_by_path or {}).get(path.resolve(), ""),
            run_id=(run_ids_by_path or {}).get(path.resolve()),
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


def initialize_local_dashboard(site_dir: Path, *, reset: bool = False) -> None:
    site_dir = site_dir.expanduser().resolve()
    prepare_site(site_dir, reset=reset)
    repo_url = repository_url()
    manifest_path = site_dir / "executions.js"
    if not manifest_path.is_file():
        manifest = {
            "schema_version": 3,
            "mode": "local",
            "last_update": "",
            "repo_url": repo_url,
            "retention": {"summaries": 100, "details": 30},
            "executions": [],
        }
        manifest_path.write_text(
            MANIFEST_PREFIX
            + json.dumps(manifest, indent=2, sort_keys=True, ensure_ascii=False)
            + ";\n"
        )
    else:
        manifest = load_manifest(manifest_path)
    write_benchmark_data(site_dir, repo_url, str(manifest.get("last_update") or ""))


def _required_string(
    payload: dict[str, Any],
    field: str,
    *,
    default: str = "",
    maximum: int = 200,
) -> str:
    value = payload.get(field, default)
    if not isinstance(value, str):
        raise LocalDashboardError(f"{field} must be a string")
    value = value.strip()
    if not value:
        raise LocalDashboardError(f"{field} is required")
    if len(value) > maximum or any(ord(character) < 32 for character in value):
        raise LocalDashboardError(f"{field} is invalid")
    return value


def _optional_string(
    payload: dict[str, Any],
    field: str,
    *,
    default: str = "",
    maximum: int = 200,
) -> str:
    value = payload.get(field, default)
    if value is None:
        return ""
    if not isinstance(value, str):
        raise LocalDashboardError(f"{field} must be a string")
    value = value.strip()
    if len(value) > maximum or any(ord(character) < 32 for character in value):
        raise LocalDashboardError(f"{field} is invalid")
    return value


def _bounded_integer(
    payload: dict[str, Any],
    field: str,
    *,
    default: int,
    minimum: int,
    maximum: int,
) -> int:
    value = payload.get(field, default)
    if isinstance(value, bool) or not isinstance(value, int):
        raise LocalDashboardError(f"{field} must be an integer")
    if not minimum <= value <= maximum:
        raise LocalDashboardError(
            f"{field} must be between {minimum} and {maximum}"
        )
    return value


def validate_stack_url(url: str) -> str:
    try:
        parsed_url = urlparse(url)
        hostname = parsed_url.hostname
    except ValueError as exc:
        raise LocalDashboardError("url must be a ws:// or wss:// endpoint") from exc
    if parsed_url.scheme not in {"ws", "wss"} or not hostname:
        raise LocalDashboardError("url must be a ws:// or wss:// endpoint")
    if parsed_url.username or parsed_url.password:
        raise LocalDashboardError("url must not contain credentials")
    return url


def validate_run_request(
    payload: dict[str, Any],
    *,
    environment: dict[str, str] | None = None,
) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise LocalDashboardError("request body must be a JSON object")
    env = environment if environment is not None else os.environ
    url = _required_string(
        payload,
        "url",
        default=env.get("III_URL", "ws://127.0.0.1:49134"),
        maximum=500,
    )
    url = validate_stack_url(url)

    scenarios = payload.get("scenarios", [])
    if not isinstance(scenarios, list) or len(scenarios) > 100:
        raise LocalDashboardError("scenarios must be an array with at most 100 items")
    normalized_scenarios = []
    for value in scenarios:
        if not isinstance(value, str) or not SCENARIO_ID.fullmatch(value.strip()):
            raise LocalDashboardError(f"invalid scenario id: {value!r}")
        scenario_id = value.strip()
        if scenario_id not in normalized_scenarios:
            normalized_scenarios.append(scenario_id)

    return {
        "url": url,
        "model": _required_string(
            payload,
            "model",
            default=env.get("HARNESS_E2E_MODEL", ""),
        ),
        "provider": _required_string(
            payload,
            "provider",
            default=env.get("HARNESS_E2E_PROVIDER", ""),
        ),
        "judge_model": _optional_string(
            payload,
            "judge_model",
            default=env.get("HARNESS_E2E_JUDGE_MODEL", ""),
        ),
        "judge_provider": _optional_string(
            payload,
            "judge_provider",
            default=env.get("HARNESS_E2E_JUDGE_PROVIDER", ""),
        ),
        "label": _optional_string(payload, "label", maximum=120),
        "runs": _bounded_integer(
            payload, "runs", default=1, minimum=1, maximum=20
        ),
        "technical_retries": _bounded_integer(
            payload,
            "technical_retries",
            default=1,
            minimum=0,
            maximum=3,
        ),
        "scenarios": normalized_scenarios,
    }


def harness_e2e_command(
    *arguments: str,
    environment: Mapping[str, str] | None = None,
) -> list[str]:
    env = os.environ if environment is None else environment
    configured = env.get("HARNESS_E2E_BIN", "").strip()
    if configured:
        candidates = [Path(configured).expanduser()]
    else:
        candidates = [DEFAULT_RUNNER_DEBUG, DEFAULT_RUNNER_RELEASE]
        path_runner = shutil.which("harness-e2e")
        if path_runner:
            candidates.append(Path(path_runner))

    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return [str(candidate.resolve()), *arguments]

    if not configured and env.get("HARNESS_E2E_ALLOW_BUILD", "").lower() in {
        "1",
        "true",
        "yes",
    }:
        return [
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "--manifest-path",
            str(REPO_ROOT / "harness" / "Cargo.toml"),
            "-p",
            "harness-e2e",
            "--",
            *arguments,
        ]

    expected = configured or str(DEFAULT_RUNNER_DEBUG)
    raise LocalDashboardError(
        f"Harness E2E runner not found at {expected}; build it once, set "
        "HARNESS_E2E_BIN, or opt into cargo with HARNESS_E2E_ALLOW_BUILD=1"
    )


def build_run_command(request: dict[str, Any], output_dir: Path) -> list[str]:
    command = harness_e2e_command(
        "run",
        "--url",
        request["url"],
        "--model",
        request["model"],
        "--provider",
        request["provider"],
        "--output",
        str(output_dir),
        "--runs",
        str(request["runs"]),
        "--technical-retries",
        str(request["technical_retries"]),
    )
    for field, flag in (
        ("judge_model", "--judge-model"),
        ("judge_provider", "--judge-provider"),
    ):
        if request[field]:
            command.extend([flag, request[field]])
    for scenario_id in request["scenarios"]:
        command.extend(["--scenario", scenario_id])
    return command


def _catalog_json(arguments: tuple[str, ...], label: str) -> Any:
    try:
        result = subprocess.run(
            harness_e2e_command(*arguments),
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=CATALOG_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as exc:
        raise LocalDashboardError(f"timed out while loading {label}") from exc
    except OSError as exc:
        raise LocalDashboardError(f"cannot load {label}: {exc}") from exc
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().splitlines()
        message = detail[-1] if detail else f"runner exited with {result.returncode}"
        raise LocalDashboardError(f"cannot load {label}: {message}")
    output = result.stdout.strip()
    try:
        return json.loads(output)
    except json.JSONDecodeError:
        # The E2E binary currently writes tracing output to stdout before the
        # command payload. Keep the CLI's existing logging behavior intact and
        # accept the final JSON line as the machine-readable result.
        for line in reversed(output.splitlines()):
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                continue
    raise LocalDashboardError(f"{label} returned invalid JSON")


def load_local_catalog(url: str) -> dict[str, Any]:
    url = validate_stack_url(url.strip())
    scenarios = _catalog_json(("list",), "scenario catalog")
    models = _catalog_json(("models", "--url", url, "--json"), "model catalog")
    if not isinstance(scenarios, list) or any(
        not isinstance(value, str) or not SCENARIO_ID.fullmatch(value)
        for value in scenarios
    ):
        raise LocalDashboardError("scenario catalog has an invalid shape")
    if not isinstance(models, list) or any(
        not isinstance(value, dict)
        or not isinstance(value.get("provider"), str)
        or not value["provider"]
        or not isinstance(value.get("id"), str)
        or not value["id"]
        for value in models
    ):
        raise LocalDashboardError("model catalog has an invalid shape")
    if not models:
        raise LocalDashboardError("the running Harness has no registered models")
    return {
        "url": url,
        "models": [
            {"provider": value["provider"], "model": value["id"]}
            for value in models
        ],
        "scenarios": scenarios,
    }


class LocalRunController:
    """Own one local E2E child process and import its report when it ends."""

    def __init__(self, site_dir: Path, runs_dir: Path):
        self.site_dir = site_dir.expanduser().resolve()
        self.runs_dir = runs_dir.expanduser().resolve()
        self._lock = threading.Lock()
        self._job: dict[str, Any] | None = None
        self._process: subprocess.Popen[str] | None = None
        self._catalog_cache: dict[str, tuple[float, dict[str, Any]]] = {}

    def defaults(self) -> dict[str, Any]:
        return {
            "url": os.environ.get("III_URL", "ws://127.0.0.1:49134"),
            "model": os.environ.get("HARNESS_E2E_MODEL", ""),
            "provider": os.environ.get("HARNESS_E2E_PROVIDER", ""),
            "judge_model": os.environ.get("HARNESS_E2E_JUDGE_MODEL", ""),
            "judge_provider": os.environ.get("HARNESS_E2E_JUDGE_PROVIDER", ""),
            "runs": 1,
            "technical_retries": 1,
        }

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            job = dict(self._job) if self._job else None
        if job:
            log_path = Path(job.pop("log_path"))
            job["log"] = self._log_tail(log_path)
        return {"job": job, "defaults": self.defaults()}

    def catalog(self, url: str, *, refresh: bool = False) -> dict[str, Any]:
        url = validate_stack_url(url.strip())
        with self._lock:
            cached = self._catalog_cache.get(url)
        if (
            cached
            and not refresh
            and time.monotonic() - cached[0] < CATALOG_CACHE_SECONDS
        ):
            return cached[1]
        catalog = load_local_catalog(url)
        with self._lock:
            self._catalog_cache[url] = (time.monotonic(), catalog)
        return catalog

    @staticmethod
    def _log_tail(path: Path) -> str:
        if not path.is_file():
            return ""
        try:
            with path.open("rb") as stream:
                stream.seek(0, 2)
                size = stream.tell()
                stream.seek(max(0, size - MAX_LOG_TAIL_BYTES))
                return stream.read().decode(errors="replace")
        except OSError:
            return ""

    def start(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = validate_run_request(payload)
        with self._lock:
            if self._job and self._job["status"] in {
                "starting",
                "running",
                "cancelling",
            }:
                raise LocalDashboardError("an E2E execution is already running")
            now = datetime.now(timezone.utc)
            job_id = f"local-{now.strftime('%Y%m%dT%H%M%S')}-{uuid.uuid4().hex[:8]}"
            run_dir = self.runs_dir / job_id
            output_dir = run_dir / "results"
            log_path = run_dir / "run.log"
            command = build_run_command(request, output_dir)
            self._job = {
                "id": job_id,
                "label": request["label"],
                "status": "starting",
                "started_at": now.isoformat(),
                "completed_at": "",
                "returncode": None,
                "execution_id": "",
                "result_path": str(output_dir / "results.json"),
                "log_path": str(log_path),
                "error": "",
            }
        thread = threading.Thread(
            target=self._execute,
            args=(job_id, command, output_dir, log_path, request["label"]),
            daemon=True,
            name=f"harness-e2e-{job_id}",
        )
        thread.start()
        return self.snapshot()

    def cancel(self) -> dict[str, Any]:
        with self._lock:
            process = self._process
            job = self._job
            if not job or job["status"] not in {"starting", "running"}:
                raise LocalDashboardError("no E2E execution is running")
            job["status"] = "cancelling"
        if process and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        return self.snapshot()

    def _execute(
        self,
        job_id: str,
        command: list[str],
        output_dir: Path,
        log_path: Path,
        label: str,
    ) -> None:
        output_dir.mkdir(parents=True, exist_ok=True)
        log_path.parent.mkdir(parents=True, exist_ok=True)
        returncode: int | None = None
        execution_id = ""
        error = ""
        cancelled = False
        try:
            with log_path.open("w") as log:
                process = subprocess.Popen(
                    command,
                    cwd=REPO_ROOT,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    text=True,
                    start_new_session=True,
                )
                with self._lock:
                    self._process = process
                    if self._job and self._job["id"] == job_id:
                        cancelled = self._job["status"] == "cancelling"
                        self._job["status"] = "cancelling" if cancelled else "running"
                if cancelled and process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                returncode = process.wait()
            with self._lock:
                cancelled = bool(
                    self._job
                    and self._job["id"] == job_id
                    and self._job["status"] == "cancelling"
                )
            results_path = output_dir / "results.json"
            if not cancelled and results_path.is_file():
                execution_id = build_local_dashboard(
                    [results_path],
                    site_dir=self.site_dir,
                    labels_by_path={results_path.resolve(): label},
                    run_ids_by_path={results_path.resolve(): job_id},
                )[0]
            elif not cancelled:
                error = "E2E runner did not produce results.json; inspect the log"
        except (OSError, CollectionError, LocalDashboardError, PublishError) as exc:
            error = str(exc)
        finally:
            with self._lock:
                self._process = None
                if self._job and self._job["id"] == job_id:
                    self._job.update(
                        {
                            "status": (
                                "cancelled"
                                if cancelled
                                else "completed"
                                if execution_id
                                else "failed"
                            ),
                            "completed_at": datetime.now(timezone.utc).isoformat(),
                            "returncode": returncode,
                            "execution_id": execution_id,
                            "error": error,
                        }
                    )


class LocalDashboardRequestHandler(http.server.SimpleHTTPRequestHandler):
    controller: LocalRunController

    def _send_json(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _is_local_client(self) -> bool:
        try:
            return ipaddress.ip_address(self.client_address[0]).is_loopback
        except ValueError:
            return False

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlsplit(self.path)
        if parsed.path == "/api/local/run":
            self._send_json(200, self.controller.snapshot())
            return
        if parsed.path == "/api/local/catalog":
            if not self._is_local_client():
                self._send_json(403, {"error": "local discovery is loopback-only"})
                return
            query = parse_qs(parsed.query)
            url = query.get("url", [self.controller.defaults()["url"]])[0]
            refresh = query.get("refresh", [""])[0] in {"1", "true"}
            try:
                self._send_json(200, self.controller.catalog(url, refresh=refresh))
            except LocalDashboardError as exc:
                self._send_json(503, {"error": str(exc)})
            return
        super().do_GET()

    def do_POST(self) -> None:  # noqa: N802
        path = self.path.split("?", 1)[0]
        if path not in {"/api/local/run", "/api/local/run/cancel"}:
            self._send_json(404, {"error": "not found"})
            return
        if not self._is_local_client():
            self._send_json(403, {"error": "local execution is loopback-only"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length < 0 or length > MAX_REQUEST_BYTES:
                raise LocalDashboardError("request body is too large")
            raw = self.rfile.read(length) if length else b"{}"
            payload = json.loads(raw)
            if path.endswith("/cancel"):
                result = self.controller.cancel()
            else:
                result = self.controller.start(payload)
            self._send_json(202, result)
        except LocalDashboardError as exc:
            self._send_json(409, {"error": str(exc)})
        except (json.JSONDecodeError, UnicodeDecodeError, ValueError):
            self._send_json(400, {"error": "request body must be valid JSON"})


def serve(
    site_dir: Path,
    host: str,
    port: int,
    *,
    runs_dir: Path = DEFAULT_RUNS_DIR,
) -> None:
    controller = LocalRunController(site_dir, runs_dir)
    handler_class = type(
        "BoundLocalDashboardRequestHandler",
        (LocalDashboardRequestHandler,),
        {"controller": controller},
    )
    handler = functools.partial(
        handler_class,
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
    parser.add_argument("--runs-dir", type=Path, default=DEFAULT_RUNS_DIR)
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
        if args.results or DEFAULT_RESULTS.is_file():
            execution_ids = build_local_dashboard(
                args.results,
                site_dir=args.site_dir,
                reset=args.reset,
            )
        else:
            initialize_local_dashboard(args.site_dir, reset=args.reset)
            execution_ids = []
        print(
            f"imported {len(execution_ids)} execution"
            f"{'' if len(execution_ids) == 1 else 's'}"
        )
        serve(
            args.site_dir.expanduser().resolve(),
            args.host,
            args.port,
            runs_dir=args.runs_dir,
        )
    except (CollectionError, LocalDashboardError, PublishError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
