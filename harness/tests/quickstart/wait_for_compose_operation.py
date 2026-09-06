#!/usr/bin/env python3
"""Wait for an admitted Compose mutation to reach a terminal state."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from collections.abc import Callable
from typing import Any


Snapshot = dict[str, Any]
FetchSnapshot = Callable[[float], Snapshot]


class ComposeOperationError(RuntimeError):
    """A Compose operation could not be observed through successful completion."""

    def __init__(self, message: str, snapshot: Snapshot | None = None) -> None:
        super().__init__(message)
        self.snapshot = snapshot


def terminal_detail(snapshot: Snapshot) -> str | None:
    last_event = snapshot.get("last_event")
    if not isinstance(last_event, dict):
        return None
    detail = last_event.get("detail")
    return detail if isinstance(detail, str) and detail else None


def wait_for_operation(
    operation_id: str,
    fetch_snapshot: FetchSnapshot,
    timeout_seconds: float,
    *,
    poll_interval_seconds: float = 0.2,
    monotonic: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
) -> Snapshot:
    """Poll an operation snapshot until it succeeds or reaches another terminal state."""

    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    if poll_interval_seconds <= 0:
        raise ValueError("poll_interval_seconds must be positive")

    deadline = monotonic() + timeout_seconds
    while True:
        remaining = deadline - monotonic()
        if remaining <= 0:
            raise ComposeOperationError(
                f"compose operation {operation_id} did not finish within {timeout_seconds:g}s"
            )

        snapshot = fetch_snapshot(remaining)
        observed_id = snapshot.get("operation_id")
        if observed_id != operation_id:
            raise ComposeOperationError(
                f"compose operation correlation mismatch: expected {operation_id}, got {observed_id!r}",
                snapshot,
            )

        status = snapshot.get("status")
        if status == "succeeded":
            return snapshot
        if status in {"failed", "cancelled"}:
            detail = terminal_detail(snapshot)
            suffix = f": {detail}" if detail else ""
            raise ComposeOperationError(
                f"compose operation {operation_id} {status}{suffix}", snapshot
            )
        if status != "running":
            raise ComposeOperationError(
                f"compose operation {operation_id} returned unexpected status {status!r}",
                snapshot,
            )

        sleep(min(poll_interval_seconds, max(0.0, deadline - monotonic())))


def fetch_operation(
    iii_bin: str, engine_port: int, operation_id: str, remaining_seconds: float
) -> Snapshot:
    """Read one operation snapshot through the published CLI."""

    request_timeout_seconds = max(0.1, min(10.0, remaining_seconds))
    request_timeout_ms = max(1, int(request_timeout_seconds * 1_000))
    payload = json.dumps({"operation_id": operation_id}, separators=(",", ":"))
    try:
        completed = subprocess.run(
            [
                iii_bin,
                "trigger",
                "compose::operation",
                "--port",
                str(engine_port),
                "--timeout-ms",
                str(request_timeout_ms),
                "--json",
                payload,
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=request_timeout_seconds,
        )
    except subprocess.TimeoutExpired as error:
        raise ComposeOperationError(
            f"timed out reading compose operation {operation_id}"
        ) from error

    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "no CLI output"
        raise ComposeOperationError(
            f"could not read compose operation {operation_id}: {detail}"
        )

    try:
        snapshot = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ComposeOperationError(
            f"compose operation {operation_id} returned invalid JSON: {error.msg}"
        ) from error
    if not isinstance(snapshot, dict):
        raise ComposeOperationError(
            f"compose operation {operation_id} returned a non-object snapshot"
        )
    return snapshot


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iii-bin", required=True)
    parser.add_argument("--engine-port", required=True, type=int)
    parser.add_argument("--operation-id", required=True)
    parser.add_argument("--timeout-seconds", required=True, type=float)
    args = parser.parse_args()

    try:
        snapshot = wait_for_operation(
            args.operation_id,
            lambda remaining: fetch_operation(
                args.iii_bin, args.engine_port, args.operation_id, remaining
            ),
            args.timeout_seconds,
        )
    except (ComposeOperationError, ValueError) as error:
        if isinstance(error, ComposeOperationError) and error.snapshot is not None:
            print(json.dumps(error.snapshot, indent=2, sort_keys=True))
        print(f"[FAIL] {error}", file=sys.stderr)
        return 1

    print(json.dumps(snapshot, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
