#!/usr/bin/env python3
"""Smoke-test released workers through an isolated iii Compose project."""

from __future__ import annotations

import argparse
import json
import subprocess
import tempfile
import tomllib
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
EXPERIMENTAL_WORKERS = {
    "a2ui",
    "canvas",
    "document",
    "eval",
    "pdf",
    "provider-opencode-go",
    "provider-openrouter",
}
REGISTRY_COMPOSE_UNSUPPORTED_WORKERS = {
    # These workers publish GitHub binary artifacts, not Registry packages.
    "acp",
    "lsp",
}
HARNESS_SELECTOR = "harness@next"


def stable_workers() -> list[str]:
    workers = sorted(
        manifest.parent.name
        for manifest in REPO_ROOT.glob("*/iii.worker.yaml")
        if manifest.parent.name not in EXPERIMENTAL_WORKERS
        and manifest.parent.name not in REGISTRY_COMPOSE_UNSUPPORTED_WORKERS
        and manifest.parent.name != "harness"
    )
    if (REPO_ROOT / "harness" / "iii.worker.yaml").is_file():
        workers.append(HARNESS_SELECTOR)
    return workers


def state_version() -> str:
    manifest = tomllib.loads(
        (REPO_ROOT / "state" / "Cargo.toml").read_text(encoding="utf-8")
    )
    return str(manifest["package"]["version"])


def compose_text(project_namespace: str) -> str:
    return f"""namespace: {project_namespace}
startup_timeout: 15m
stop_timeout: 10s

containers:
  # Bootstrap entry required by the compose::add text editor.
  state:
    worker: package://api.workers.iii.dev/state
    version: {state_version()}
"""


def parse_cli_error(output: str) -> dict[str, Any]:
    value = output.strip()
    if value.startswith("Error: "):
        value = value.removeprefix("Error: ")
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        return {"code": "CLI_ERROR", "message": value}
    if isinstance(parsed, dict):
        return parsed
    return {"code": "CLI_ERROR", "message": value}


def trigger(
    daemon_namespace: str,
    function_id: str,
    payload: dict[str, Any],
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    command = [
        "iii",
        "trigger",
        "-n",
        daemon_namespace,
        function_id,
        "--json",
        json.dumps(payload),
        "--timeout-ms",
        "900000",
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=930,
        )
    except subprocess.TimeoutExpired:
        return None, {
            "code": "CLI_TIMEOUT",
            "message": f"{function_id} exceeded 930 seconds",
        }

    output = completed.stdout.strip()
    if completed.returncode != 0:
        return None, parse_cli_error(completed.stderr or output)
    try:
        result = json.loads(output)
    except json.JSONDecodeError:
        return None, parse_cli_error(output)
    if not isinstance(result, dict):
        return None, {"code": "INVALID_RESPONSE", "message": output}
    return result, None


def lifecycle_errors(result: dict[str, Any]) -> list[dict[str, str]]:
    errors: list[dict[str, str]] = []
    for container in result.get("up", {}).get("containers", []):
        error = container.get("error")
        if not isinstance(error, dict):
            continue
        errors.append(
            {
                "container": str(container.get("container", "unknown")),
                "code": str(error.get("code", "UNKNOWN")),
                "message": str(error.get("message", "unknown lifecycle error")),
            }
        )
    return errors


def target_ready(result: dict[str, Any], worker: str) -> bool:
    return any(
        container.get("container") == worker and container.get("state") == "ready"
        for container in result.get("up", {}).get("containers", [])
    )


def worker_key(worker_spec: str) -> str:
    name = worker_spec.rsplit("/", 1)[-1]
    return name.rsplit("@", 1)[0]


def ordered_workers(worker_specs: list[str]) -> list[str]:
    """Keep Harness last and always test its current candidate channel."""
    workers = [spec for spec in worker_specs if worker_key(spec) != "harness"]
    if len(workers) != len(worker_specs):
        workers.append(HARNESS_SELECTOR)
    return workers


def test_worker(daemon_namespace: str, worker_spec: str) -> dict[str, Any]:
    worker = worker_key(worker_spec)
    with tempfile.TemporaryDirectory(prefix=f"iii-registry-{worker}-") as temp_dir:
        compose_file = Path(temp_dir) / "worker-compose.yaml"
        compose_file.write_text(
            compose_text(f"registry-test-{worker}"), encoding="utf-8"
        )
        payload = {"file": str(compose_file)}

        try:
            initial, error = trigger(daemon_namespace, "compose::up", payload)
            if error:
                return {
                    "worker": worker_spec,
                    "status": "fail",
                    "errors": [error],
                }
            if worker == "state":
                initial = initial or {}
                result = {"status": initial.get("status"), "up": initial}
            else:
                result, error = trigger(
                    daemon_namespace,
                    "compose::add",
                    {**payload, "worker": worker_spec},
                )
                if error:
                    return {
                        "worker": worker_spec,
                        "status": "fail",
                        "errors": [error],
                    }
                result = result or {}

            errors = lifecycle_errors(result)
            passed = result.get("status") == "ok" and target_ready(result, worker)
            return {
                "worker": worker_spec,
                "status": "pass" if passed else "fail",
                "errors": []
                if passed
                else errors
                or [
                    {
                        "code": "NOT_READY",
                        "message": f"compose returned {result.get('status')!r}",
                    }
                ],
            }
        finally:
            trigger(daemon_namespace, "compose::down", payload)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Test Registry workers through iii Compose"
    )
    parser.add_argument("workers", nargs="*", help="Workers to test")
    parser.add_argument(
        "--namespace",
        default="a",
        help="Namespace used by the running Compose daemon",
    )
    args = parser.parse_args()

    workers = ordered_workers(args.workers) if args.workers else stable_workers()
    results: list[dict[str, Any]] = []
    for worker in workers:
        result = test_worker(args.namespace, worker)
        results.append(result)
        if result["status"] == "pass":
            print(f"PASS\t{worker}", flush=True)
            continue
        reason = "; ".join(
            f"{error.get('container', worker)}: {error['code']}: {error['message']}"
            for error in result["errors"]
        )
        print(f"FAIL\t{worker}\t{reason}", flush=True)

    failures = [result for result in results if result["status"] == "fail"]
    print(json.dumps({"results": results}, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
