#!/usr/bin/env python3
"""Publish a worker and reconcile ambiguous Registry responses."""

from __future__ import annotations

import argparse
import json
import os
import urllib.error
from pathlib import Path
from typing import Any

from registry_release import RegistryError, request_json, resolve_version


def reconcile_publish(
    api_url: str,
    worker: str,
    version: str,
    registry_tag: str,
    reason: str,
) -> dict[str, Any]:
    try:
        resolved = resolve_version(api_url, worker, registry_tag, allow_missing=True)
    except (RegistryError, TimeoutError, OSError, urllib.error.URLError) as error:
        raise RegistryError(
            f"{reason}; Registry reconciliation also failed: {error}; publication state is unknown"
        ) from error
    if resolved != version:
        raise RegistryError(
            f"{reason}; {worker}@{registry_tag} resolves {resolved or 'nothing'}, "
            f"expected {version}; publication state is unknown"
        )
    return {
        "worker": worker,
        "version": version,
        "registry_tag": registry_tag,
        "resolved_version": resolved,
        "reconciled": True,
        "reason": reason,
    }


def publish(
    api_url: str,
    api_key: str,
    payload: dict[str, Any],
    worker: str,
    version: str,
    registry_tag: str,
) -> dict[str, Any]:
    try:
        status, response = request_json(
            "POST",
            f"{api_url.rstrip('/')}/publish",
            payload,
            api_key=api_key,
        )
    except (json.JSONDecodeError, UnicodeDecodeError, TimeoutError, OSError, urllib.error.URLError) as error:
        return reconcile_publish(
            api_url,
            worker,
            version,
            registry_tag,
            f"publish transport failed: {error}",
        )

    if status == 200:
        return {
            "worker": worker,
            "version": version,
            "registry_tag": registry_tag,
            "reconciled": False,
            "registry_response": response,
        }
    if status == 409:
        return reconcile_publish(
            api_url,
            worker,
            version,
            registry_tag,
            "Registry reported a duplicate publish",
        )
    raise RegistryError(f"publish failed with HTTP {status}: {json.dumps(response, sort_keys=True)}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--registry-tag", choices=("next", "latest"), required=True)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    api_key = os.environ.get("WORKERS_REGISTRY_API_KEY", "")
    if not api_key:
        raise SystemExit("WORKERS_REGISTRY_API_KEY is required")
    try:
        result = publish(
            args.api_url,
            api_key,
            json.loads(args.payload.read_text()),
            args.worker,
            args.version,
            args.registry_tag,
        )
    except RegistryError as error:
        print(f"::error::{error}")
        raise SystemExit(1) from error
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
