#!/usr/bin/env python3
"""Find the non-expired Harness E2E runner artifact for an exact commit."""

from __future__ import annotations

import argparse
import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


class LookupError(RuntimeError):
    """Raised when the runner cannot be resolved safely."""


def api_json(url: str, token: str) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            value = json.load(response)
    except (urllib.error.URLError, json.JSONDecodeError) as error:
        raise LookupError(f"GitHub API request failed for {url}: {error}") from error
    if not isinstance(value, dict):
        raise LookupError(f"GitHub API returned a non-object for {url}")
    return value


def find_runner(
    *,
    api_url: str,
    repository: str,
    workflow: str,
    source_sha: str,
    token: str,
) -> dict[str, Any]:
    artifact_name = f"harness-e2e-runner-{source_sha}"
    query = urllib.parse.urlencode(
        {"head_sha": source_sha, "event": "push", "per_page": 100}
    )
    runs_url = (
        f"{api_url}/repos/{repository}/actions/workflows/{workflow}/runs?{query}"
    )
    runs = api_json(runs_url, token).get("workflow_runs")
    if not isinstance(runs, list):
        raise LookupError("GitHub API response did not contain workflow_runs")

    ordered_runs = sorted(
        (run for run in runs if isinstance(run, dict) and run.get("id")),
        key=lambda run: str(run.get("created_at", "")),
        reverse=True,
    )
    for run in ordered_runs:
        run_id = int(run["id"])
        artifacts_url = (
            f"{api_url}/repos/{repository}/actions/runs/{run_id}/artifacts?per_page=100"
        )
        artifacts = api_json(artifacts_url, token).get("artifacts")
        if not isinstance(artifacts, list):
            continue
        for artifact in artifacts:
            if (
                isinstance(artifact, dict)
                and artifact.get("name") == artifact_name
                and artifact.get("expired") is False
            ):
                return {
                    "run_id": run_id,
                    "artifact_id": artifact.get("id"),
                    "artifact_name": artifact_name,
                }

    raise LookupError(
        f"no non-expired {artifact_name} artifact was found; "
        "the daily workflow will not compile a fallback"
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api-url", default="https://api.github.com")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--workflow", default="harness-e2e-main.yml")
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--token", required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        runner = find_runner(
            api_url=args.api_url.rstrip("/"),
            repository=args.repository,
            workflow=args.workflow,
            source_sha=args.source_sha,
            token=args.token,
        )
    except LookupError as error:
        raise SystemExit(f"runner_not_found: {error}") from error
    print(json.dumps(runner, sort_keys=True))


if __name__ == "__main__":
    main()
