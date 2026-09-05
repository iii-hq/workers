import importlib.util
import json
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest


ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "harness" / "tests" / "quickstart" / "wait_for_compose_operation.py"
SPEC = importlib.util.spec_from_file_location("wait_for_compose_operation", HELPER)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Clock:
    def __init__(self) -> None:
        self.now = 0.0

    def monotonic(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += seconds


def snapshot(operation_id: str, status: str, detail: str = "") -> dict:
    return {
        "operation_id": operation_id,
        "status": status,
        "last_event": {"detail": detail} if detail else None,
    }


def test_waits_through_running_until_succeeded():
    operation_id = "compose:success"
    responses = iter(
        [
            snapshot(operation_id, "running"),
            snapshot(operation_id, "running"),
            snapshot(operation_id, "succeeded", "workers are ready"),
        ]
    )
    clock = Clock()

    result = MODULE.wait_for_operation(
        operation_id,
        lambda _remaining: next(responses),
        5,
        poll_interval_seconds=0.25,
        monotonic=clock.monotonic,
        sleep=clock.sleep,
    )

    assert result["status"] == "succeeded"
    assert clock.now == 0.5


@pytest.mark.parametrize("status", ["failed", "cancelled"])
def test_rejects_terminal_non_success_with_detail(status: str):
    operation_id = f"compose:{status}"

    with pytest.raises(
        MODULE.ComposeOperationError, match=f"{status}: registry unavailable"
    ) as raised:
        MODULE.wait_for_operation(
            operation_id,
            lambda _remaining: snapshot(operation_id, status, "registry unavailable"),
            5,
        )

    assert raised.value.snapshot["status"] == status


def test_times_out_while_operation_remains_running():
    operation_id = "compose:slow"
    clock = Clock()

    with pytest.raises(MODULE.ComposeOperationError, match="did not finish within 1s"):
        MODULE.wait_for_operation(
            operation_id,
            lambda _remaining: snapshot(operation_id, "running"),
            1,
            poll_interval_seconds=0.25,
            monotonic=clock.monotonic,
            sleep=clock.sleep,
        )

    assert clock.now == 1


def test_rejects_snapshot_for_a_different_operation():
    with pytest.raises(MODULE.ComposeOperationError, match="correlation mismatch"):
        MODULE.wait_for_operation(
            "compose:expected",
            lambda _remaining: snapshot("compose:other", "succeeded"),
            5,
        )


def test_fetches_snapshot_through_compose_operation(monkeypatch):
    calls = []

    def fake_run(command, **kwargs):
        calls.append((command, kwargs))
        return SimpleNamespace(
            returncode=0,
            stdout=json.dumps(snapshot("compose:123", "running")),
            stderr="",
        )

    monkeypatch.setattr(MODULE.subprocess, "run", fake_run)

    result = MODULE.fetch_operation("/tmp/iii", 49134, "compose:123", 12)

    assert result["status"] == "running"
    assert calls == [
        (
            [
                "/tmp/iii",
                "trigger",
                "compose::operation",
                "--port",
                "49134",
                "--timeout-ms",
                "10000",
                "--json",
                '{"operation_id":"compose:123"}',
            ],
            {
                "check": False,
                "capture_output": True,
                "text": True,
                "timeout": 10.0,
            },
        )
    ]


def test_reports_cli_failure_without_accepting_stdout(monkeypatch):
    monkeypatch.setattr(
        MODULE.subprocess,
        "run",
        lambda *_args, **_kwargs: SimpleNamespace(
            returncode=1,
            stdout='{"status":"succeeded"}',
            stderr="operation not found",
        ),
    )

    with pytest.raises(MODULE.ComposeOperationError, match="operation not found"):
        MODULE.fetch_operation("/tmp/iii", 49134, "compose:missing", 1)


def test_reports_cli_timeout(monkeypatch):
    def timeout(*_args, **_kwargs):
        raise subprocess.TimeoutExpired("iii", 1)

    monkeypatch.setattr(MODULE.subprocess, "run", timeout)

    with pytest.raises(MODULE.ComposeOperationError, match="timed out reading"):
        MODULE.fetch_operation("/tmp/iii", 49134, "compose:slow", 1)
