#!/usr/bin/env python3
"""Promote a staged worker release through the Registry HTTP API."""

from __future__ import annotations

import argparse
import json
import os
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


class RegistryError(RuntimeError):
    pass


def request_json(
    method: str,
    url: str,
    payload: dict[str, Any],
    *,
    api_key: str | None = None,
) -> tuple[int, dict[str, Any]]:
    body = json.dumps(payload).encode()
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["X-API-Key"] = api_key
    request = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, json.loads(response.read().decode())
    except urllib.error.HTTPError as error:
        try:
            response = json.loads(error.read().decode())
        except (json.JSONDecodeError, UnicodeDecodeError):
            response = {"error": f"HTTP {error.code}"}
        return error.code, response


def resolved_root_version(response: dict[str, Any]) -> str:
    root = response.get("root")
    version = root.get("version") if isinstance(root, dict) else None
    if not isinstance(version, str) or not version:
        raise RegistryError("Registry resolve response has no root.version")
    return version


def resolve_version(api_url: str, worker: str, selector: str, *, allow_missing: bool = False) -> str | None:
    status, response = request_json(
        "POST",
        f"{api_url.rstrip('/')}/resolve",
        {"worker": worker, "version": selector},
    )
    if status == 200:
        return resolved_root_version(response)
    error = response.get("error")
    code = error.get("code") if isinstance(error, dict) else None
    if allow_missing and code in {"version_not_found", "worker_not_found"}:
        return None
    raise RegistryError(f"resolve {worker}@{selector} failed with HTTP {status}: {json.dumps(response)}")


def promotion_payload(version: str, current_latest: str | None) -> dict[str, str]:
    payload = {"version": version, "expected_tag": "next"}
    if current_latest is not None:
        payload["expected_current_version"] = current_latest
    return payload


def promote(
    api_url: str,
    api_key: str,
    worker: str,
    version: str,
    expected_next: str,
    expected_latest: str | None,
) -> dict[str, Any]:
    current_latest = resolve_version(api_url, worker, "latest", allow_missing=True)
    current_next = resolve_version(api_url, worker, "next", allow_missing=True)
    if current_next != expected_next:
        raise RegistryError(f"next points to {current_next}, expected {expected_next}")
    if expected_next != version:
        raise RegistryError(f"promotion target {version} does not match expected next {expected_next}")
    if current_latest == version:
        return {
            "worker": worker,
            "version": version,
            "previous_latest": current_latest,
            "next": current_next,
            "latest": current_latest,
            "changed": False,
            "registry_response": {"changed": False, "idempotent": True},
        }
    if current_latest != expected_latest:
        raise RegistryError(f"latest points to {current_latest}, expected {expected_latest}")
    encoded_worker = urllib.parse.quote(worker, safe="")
    status, response = request_json(
        "PUT",
        f"{api_url.rstrip('/')}/w/{encoded_worker}/tags/latest",
        promotion_payload(version, current_latest),
        api_key=api_key,
    )
    if status != 200:
        raise RegistryError(f"promotion failed with HTTP {status}: {json.dumps(response)}")

    promoted = resolve_version(api_url, worker, "latest")
    if promoted != version:
        raise RegistryError(f"promotion verification resolved {promoted}, expected {version}")

    return {
        "worker": worker,
        "version": version,
        "previous_latest": current_latest,
        "next": current_next,
        "latest": promoted,
        "changed": bool(response.get("changed")),
        "registry_response": response,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--expected-next-version", required=True)
    parser.add_argument("--expected-latest-version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    api_key = os.environ.get("WORKERS_REGISTRY_API_KEY", "")
    if not api_key:
        raise SystemExit("WORKERS_REGISTRY_API_KEY is required")
    try:
        expected_latest = None if args.expected_latest_version == "none" else args.expected_latest_version
        result = promote(
            args.api_url,
            api_key,
            args.worker,
            args.version,
            args.expected_next_version,
            expected_latest,
        )
    except RegistryError as error:
        raise SystemExit(str(error)) from error
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
