"""Tests for the observer read in collect_worker_interface.py.

CI reads `engine::observers::list` for the observers a booted worker created
(`--observers-out`) and fails when one of them is in state `error`
(`--assert-observers`).
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from collect_worker_interface import (  # noqa: E402
    _observer_violations,
    _observer_warnings,
    collect_worker_observers,
)

SCRIPT = Path(__file__).resolve().parents[1] / "collect_worker_interface.py"


def observer(
    observer_id: str,
    *,
    function_id: str = "configuration::on-updated",
    target: str = "approval::on-config-change",
    state: str = "active",
    reason: dict | None = None,
    worker_name: str = "approval-gate",
) -> dict:
    """One `engine::observers::list` row (iii 0.26.0 `ObserverInfo`)."""
    return {
        "id": observer_id,
        "function_id": function_id,
        "namespace": "default",
        "args": None,
        "filters": {"input.id": {"$eq": "approval-gate"}},
        "deliver": "output",
        "capture": [],
        "on": "call",
        "target": {"function_id": target, "namespace": "default"},
        "worker": {"worker_id": "6a1c", "worker_name": worker_name},
        "state": state,
        "reason": reason,
        "fires": 0,
        "dropped": {},
        "activation_failures": {},
        "created_at_ms": 1,
        "max_in_flight": 1,
    }


def test_reads_the_observers_each_target_worker_created() -> None:
    calls: list[tuple[str, dict]] = []
    listed = {
        "approval-gate": [observer("o-2"), observer("o-1")],
        "approval-gate-ui": [observer("o-3", worker_name="approval-gate-ui"), observer("o-1")],
    }

    def run(function_path: str, payload: dict) -> dict:
        calls.append((function_path, payload))
        return {"observers": listed[payload["worker"]]}

    observers = collect_worker_observers({"approval-gate-ui", "approval-gate"}, run=run)

    assert calls == [
        ("engine::observers::list", {"worker": "approval-gate"}),
        ("engine::observers::list", {"worker": "approval-gate-ui"}),
    ]
    assert [o["id"] for o in observers] == ["o-1", "o-2", "o-3"]


def test_a_reply_without_an_observers_array_is_an_error() -> None:
    with pytest.raises(ValueError, match="approval-gate"):
        collect_worker_observers({"approval-gate"}, run=lambda _path, _payload: {})


def test_an_observer_in_error_is_a_violation() -> None:
    rejected = observer(
        "o-1",
        function_id="harness::hook::on-pre-call",
        target="approval::gate",
        state="error",
        reason={"code": "OBSERVE_REJECTED", "message": "unknown hook point"},
    )

    assert _observer_violations([observer("o-0"), rejected]) == [
        "o-1 (harness::hook::on-pre-call -> approval::gate): "
        "OBSERVE_REJECTED: unknown hook point"
    ]


@pytest.mark.parametrize(
    "state, code",
    [
        ("active", None),
        # The observed function's host is not running: the smoke boots one worker.
        ("pending", "OWNER_UNAVAILABLE"),
        ("pending", "TARGET_UNAVAILABLE"),
        ("suspended", "TARGET_UNAVAILABLE"),
    ],
)
def test_observers_outside_error_are_not_violations(state: str, code: str | None) -> None:
    reason = {"code": code, "message": "m"} if code else None
    assert _observer_violations([observer("o-1", state=state, reason=reason)]) == []


def test_a_pending_observer_without_its_target_is_a_warning() -> None:
    missing_target = observer(
        "o-1",
        target="approval::on-config-chnage",
        state="pending",
        reason={"code": "TARGET_UNAVAILABLE", "message": "not registered"},
    )
    no_host = observer(
        "o-2",
        function_id="console::ui::asset",
        state="pending",
        reason={"code": "OWNER_UNAVAILABLE", "message": "no worker hosts it"},
    )

    assert _observer_warnings([missing_target, no_host]) == [
        "o-1 (configuration::on-updated -> approval::on-config-chnage): "
        "TARGET_UNAVAILABLE: not registered"
    ]


def run_assert(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args], capture_output=True, text=True
    )


def write_observers(path: Path, observers: list[dict]) -> Path:
    path.write_text(json.dumps({"observers": observers}), encoding="utf-8")
    return path


class TestAssertObservers:
    def test_passes_when_no_observer_is_in_error(self, tmp_path: Path) -> None:
        observers = write_observers(tmp_path / "observers.json", [
            observer("o-1"),
            observer("o-2", function_id="cron::on-tick", state="pending",
                     reason={"code": "OWNER_UNAVAILABLE", "message": "no worker hosts it"}),
        ])

        r = run_assert("--assert-observers", str(observers))

        assert r.returncode == 0, r.stderr

    def test_passes_with_no_observers(self, tmp_path: Path) -> None:
        r = run_assert("--assert-observers", str(write_observers(tmp_path / "o.json", [])))

        assert r.returncode == 0, r.stderr

    def test_fails_when_an_observer_is_in_error(self, tmp_path: Path) -> None:
        observers = write_observers(tmp_path / "observers.json", [
            observer("o-1", state="error",
                     reason={"code": "OWNER_TIMEOUT", "message": "no ack"}),
        ])

        r = run_assert("--assert-observers", str(observers))

        assert r.returncode == 1
        assert "::error::observers in state error" in r.stderr
        assert "o-1 (configuration::on-updated -> approval::on-config-change)" in r.stderr
        assert "OWNER_TIMEOUT: no ack" in r.stderr

    def test_warns_on_a_missing_target(self, tmp_path: Path) -> None:
        observers = write_observers(tmp_path / "observers.json", [
            observer("o-1", state="pending",
                     reason={"code": "TARGET_UNAVAILABLE", "message": "not registered"}),
        ])

        r = run_assert("--assert-observers", str(observers))

        assert r.returncode == 0, r.stderr
        assert "::warning::observer target is not registered: o-1" in r.stderr

    def test_fails_without_an_observers_array(self, tmp_path: Path) -> None:
        path = tmp_path / "observers.json"
        path.write_text("{}", encoding="utf-8")

        r = run_assert("--assert-observers", str(path))

        assert r.returncode == 1
        assert "no observers array" in r.stderr

    def test_fails_on_a_missing_file(self, tmp_path: Path) -> None:
        r = run_assert("--assert-observers", str(tmp_path / "absent.json"))

        assert r.returncode == 1
        assert "could not read" in r.stderr

    def test_runs_with_the_interface_assertions(self, tmp_path: Path) -> None:
        interface = tmp_path / "interface.json"
        interface.write_text(json.dumps({"functions": [{"name": "approval::gate"}]}))
        observers = write_observers(tmp_path / "observers.json", [
            observer("o-1", state="error",
                     reason={"code": "OBSERVE_REJECTED", "message": "no"}),
        ])

        r = run_assert(
            "--assert-non-empty", "--assert-file", str(interface),
            "--assert-observers", str(observers),
        )

        assert r.returncode == 1
        assert "observers in state error" in r.stderr

    def test_an_interface_failure_still_fails_with_clean_observers(self, tmp_path: Path) -> None:
        interface = tmp_path / "interface.json"
        interface.write_text(json.dumps({"functions": []}))
        observers = write_observers(tmp_path / "observers.json", [observer("o-1")])

        r = run_assert(
            "--assert-non-empty", "--assert-file", str(interface),
            "--assert-observers", str(observers),
        )

        assert r.returncode == 1
        assert "no worker functions" in r.stderr


FAKE_III = r'''#!/usr/bin/env python3
import json
import os
import sys

args = sys.argv[1:]
assert args[0] == "call", args
function_id = args[1]
payload = json.loads(args[args.index("--json") + 1])
with open(os.environ["FAKE_III_CALLS"], "a", encoding="utf-8") as calls:
    calls.write(json.dumps([function_id, payload]) + "\n")
schema = {"type": "object", "properties": {"ok": {"type": "boolean"}}}
if function_id == "engine::workers::list":
    reply = {"workers": [{"id": "c-1", "name": "observe-smoke"}]}
elif function_id == "engine::functions::list":
    reply = {"functions": [{"function_id": "observe-smoke::ping",
                            "worker_name": "observe-smoke", "observer_count": 0}]}
elif function_id == "engine::functions::info":
    reply = {"function_id": payload["function_id"], "worker_name": "observe-smoke",
             "request_schema": schema, "response_schema": schema}
elif function_id == "engine::observers::list":
    if os.environ.get("FAKE_III_OBSERVERS_FAIL"):
        print("function engine::observers::list not found", file=sys.stderr)
        sys.exit(1)
    reply = json.loads(os.environ["FAKE_III_OBSERVERS"])
else:
    sys.exit(f"unexpected call {function_id}")
print(json.dumps(reply))
'''


def run_collect(tmp_path: Path, *, observers: list[dict] | None, fail: bool = False):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    fake = bin_dir / "iii"
    fake.write_text(FAKE_III.replace("/usr/bin/env python3", sys.executable, 1), encoding="utf-8")
    fake.chmod(0o755)
    calls = tmp_path / "calls.jsonl"
    env = {
        "PATH": f"{bin_dir}:/usr/bin:/bin",
        "FAKE_III_CALLS": str(calls),
        "FAKE_III_OBSERVERS": json.dumps({"observers": observers or []}),
    }
    if fail:
        env["FAKE_III_OBSERVERS_FAIL"] = "1"
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--worker", "observe-smoke",
         "--out", str(tmp_path / "interface.json"),
         "--observers-out", str(tmp_path / "observers.json")],
        capture_output=True, text=True, env=env, cwd=tmp_path,
    )
    lines = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result, [json.loads(line) for line in lines]


class TestObserversOut:
    def test_writes_the_observers_the_worker_created(self, tmp_path: Path) -> None:
        rows = [observer("o-2", worker_name="observe-smoke"),
                observer("o-1", worker_name="observe-smoke")]

        r, calls = run_collect(tmp_path, observers=rows)

        assert r.returncode == 0, r.stderr
        assert ["engine::observers::list", {"worker": "observe-smoke"}] in calls
        written = json.loads((tmp_path / "observers.json").read_text(encoding="utf-8"))
        assert [o["id"] for o in written["observers"]] == ["o-1", "o-2"]
        assert "2 observer(s) created by observe-smoke:" in r.stdout
        interface = json.loads((tmp_path / "interface.json").read_text(encoding="utf-8"))
        assert [f["name"] for f in interface["functions"]] == ["observe-smoke::ping"]

    def test_a_failed_read_fails_the_collection(self, tmp_path: Path) -> None:
        r, _calls = run_collect(tmp_path, observers=None, fail=True)

        assert r.returncode == 1
        assert "::error::could not read engine::observers::list" in r.stderr
        assert "function engine::observers::list not found" in r.stderr
        assert not (tmp_path / "observers.json").exists()
