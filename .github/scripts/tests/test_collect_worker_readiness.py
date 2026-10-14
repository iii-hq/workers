"""Capture must wait for asynchronous function registration to settle."""
import json
import sys

import pytest

import collect_worker_interface as collector


def registration_sequence(monkeypatch, snapshots, *, trigger_snapshots=None, workers=None):
    """Replace only engine I/O and wall time; exercise real worker resolution."""
    clock = {"now": 0.0, "index": 0}
    monkeypatch.setattr(collector.time, "monotonic", lambda: clock["now"])

    def sleep(seconds):
        clock["now"] += seconds

    def run_iii(function_id, payload):
        index = min(clock["index"], len(snapshots) - 1)
        if function_id == "engine::workers::list":
            assert payload == {}
            return {"workers": workers if workers is not None else [{"name": "whatsapp"}, {"name": "other"}]}
        if function_id == "engine::triggers::list":
            assert payload == {"include_internal": False}
            sample = min(max(clock["index"] - 1, 0), len(trigger_snapshots) - 1) if trigger_snapshots else 0
            return {"triggers": trigger_snapshots[sample] if trigger_snapshots else []}
        if function_id == "engine::triggers::info":
            return {"id": payload["id"], "configuration_schema": {"type": "object"}, "request_schema": {"type": "object"}}
        assert function_id == "engine::functions::list"
        assert payload == {"include_internal": True}
        clock["index"] += 1
        return {"functions": [
            {"function_id": name, "worker_name": worker}
            for worker, names in snapshots[index].items()
            for name in names
        ]}

    monkeypatch.setattr(collector.time, "sleep", sleep)
    monkeypatch.setattr(collector, "run_iii", run_iii)
    return clock


def names(functions):
    return [row["function_id"] for row in functions["functions"] if row["worker_name"] == "whatsapp"]


def test_capture_waits_through_gradual_registration_and_a_brief_plateau(monkeypatch):
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": ["whatsapp::first"]},
        {"whatsapp": ["whatsapp::first"]},
        {"whatsapp": ["whatsapp::first", "whatsapp::second"]},
        {"whatsapp": ["whatsapp::second", "whatsapp::first"]},
        {"whatsapp": ["whatsapp::first", "whatsapp::second"]},
    ])

    _, functions = collector.wait_for_worker("whatsapp", 20)

    assert names(functions) == ["whatsapp::first", "whatsapp::second"]
    assert clock["now"] == 4 * collector.SAMPLE_INTERVAL_SECONDS


def test_registration_ids_must_stabilize_even_when_count_is_unchanged(monkeypatch):
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": ["whatsapp::old"]},
        {"whatsapp": ["whatsapp::new"]},
        {"whatsapp": ["whatsapp::new"]},
        {"whatsapp": ["whatsapp::new"]},
    ])

    _, functions = collector.wait_for_worker("whatsapp", 20)

    assert names(functions) == ["whatsapp::new"]
    assert clock["now"] == 3 * collector.SAMPLE_INTERVAL_SECONDS


def test_other_workers_registering_do_not_prevent_target_readiness(monkeypatch):
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": ["whatsapp::ready"], "other": ["other::first"]},
        {"whatsapp": ["whatsapp::ready"], "other": ["other::first", "other::second"]},
        {"whatsapp": ["whatsapp::ready"], "other": ["other::third"]},
    ])

    _, functions = collector.wait_for_worker("whatsapp", 20)

    assert names(functions) == ["whatsapp::ready"]
    assert clock["now"] == 2 * collector.SAMPLE_INTERVAL_SECONDS


def test_capture_settles_before_a_worker_that_exits_seconds_after_registering(monkeypatch):
    # The boot smoke runs a worker with no dependencies: harness registers in
    # ~0.2 s, then exits ~5 s later without a queue worker. Three stable
    # samples must fit well inside that window.
    clock = registration_sequence(monkeypatch, [{"whatsapp": ["whatsapp::ready"]}])
    engine = collector.run_iii

    def run_iii(function_id, payload):
        result = engine(function_id, payload)
        if function_id == "engine::functions::list" and clock["now"] >= 3:
            return {"functions": []}
        return result

    monkeypatch.setattr(collector, "run_iii", run_iii)

    _, functions = collector.wait_for_worker("whatsapp", 20)

    assert names(functions) == ["whatsapp::ready"]


@pytest.mark.parametrize("wait_seconds", [0, -1])
def test_no_wait_preserves_immediate_snapshot(monkeypatch, wait_seconds):
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": ["whatsapp::first"]},
        {"whatsapp": ["whatsapp::first", "whatsapp::second"]},
    ])

    _, functions = collector.wait_for_worker("whatsapp", wait_seconds)

    assert names(functions) == ["whatsapp::first"]
    assert clock["now"] == 0


def test_absent_registration_times_out_without_oversleeping(monkeypatch):
    clock = registration_sequence(monkeypatch, [{"other": ["other::ready"]}])

    with pytest.raises(TimeoutError, match="whatsapp.*stabilize"):
        collector.wait_for_worker("whatsapp", 3)

    assert clock["now"] == 3


def test_changing_registration_times_out_instead_of_publishing_partial_interface(monkeypatch):
    samples = int(5 / collector.SAMPLE_INTERVAL_SECONDS) + 1
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": [f"whatsapp::fn{i}" for i in range(n + 1)]} for n in range(samples)
    ])

    with pytest.raises(TimeoutError, match="whatsapp.*stabilize"):
        collector.wait_for_worker("whatsapp", 5)

    assert clock["now"] == 5


def test_cli_reports_registration_timeout_without_writing_interface(monkeypatch, tmp_path, capsys):
    registration_sequence(monkeypatch, [{"whatsapp": []}])
    output = tmp_path / "interface.json"
    monkeypatch.setattr(sys, "argv", [
        "collect_worker_interface.py", "--worker", "whatsapp", "--wait-seconds", "3", "--out", str(output),
    ])

    assert collector.main() == 1
    assert "::error::" in capsys.readouterr().err
    assert not output.exists()


def test_trigger_only_capture_waits_for_all_registrations_and_publishes_them(monkeypatch, tmp_path):
    first = {"id": "whatsapp::first-event", "worker_name": "whatsapp", "namespace": "default"}
    second = {"id": "whatsapp::second-event", "worker_name": "whatsapp", "namespace": "default"}
    clock = registration_sequence(monkeypatch, [{"whatsapp": []}], trigger_snapshots=[
        [first], [first], [first, second], [second, first], [first, second],
    ])
    output = tmp_path / "interface.json"
    monkeypatch.setattr(sys, "argv", [
        "collect_worker_interface.py", "--worker", "whatsapp", "--wait-seconds", "20", "--out", str(output),
        "--assert-non-empty",
    ])

    assert collector.main() == 0
    interface = json.loads(output.read_text())
    assert interface["functions"] == []
    assert [row["name"] for row in interface["triggers"]] == ["whatsapp::first-event", "whatsapp::second-event"]
    assert clock["now"] == 20


def test_other_workers_triggers_cannot_make_an_empty_target_ready(monkeypatch):
    clock = registration_sequence(monkeypatch, [{"whatsapp": []}], trigger_snapshots=[[
        {"id": "other::ready", "worker_name": "other", "namespace": "default"},
    ]])

    with pytest.raises(TimeoutError, match="whatsapp.*stabilize"):
        collector.wait_for_worker("whatsapp", 5)

    assert clock["now"] == 5


def test_trigger_registrations_cannot_make_an_absent_worker_ready(monkeypatch):
    clock = registration_sequence(monkeypatch, [{"whatsapp": []}], workers=[], trigger_snapshots=[[
        {"id": "whatsapp::event", "worker_name": "whatsapp", "namespace": "default"},
    ]])

    with pytest.raises(TimeoutError, match="whatsapp.*stabilize"):
        collector.wait_for_worker("whatsapp", 5)

    assert clock["now"] == 5


def test_delayed_functions_reset_trigger_only_readiness(monkeypatch):
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": []}, {"whatsapp": []}, {"whatsapp": ["whatsapp::first"]},
        {"whatsapp": ["whatsapp::first", "whatsapp::second"]},
        {"whatsapp": ["whatsapp::first", "whatsapp::second"]},
        {"whatsapp": ["whatsapp::first", "whatsapp::second"]},
    ], trigger_snapshots=[[
        {"id": "whatsapp::event", "worker_name": "whatsapp", "namespace": "default"},
    ]])

    _, functions = collector.wait_for_worker("whatsapp", 20)

    assert names(functions) == ["whatsapp::first", "whatsapp::second"]
    assert clock["now"] == 5 * collector.SAMPLE_INTERVAL_SECONDS


def test_trigger_only_readiness_cannot_hide_functions_registered_after_a_long_pause(monkeypatch):
    clock = registration_sequence(monkeypatch, [
        {"whatsapp": []}, {"whatsapp": []}, {"whatsapp": []},
        {"whatsapp": ["whatsapp::ready"]}, {"whatsapp": ["whatsapp::ready"]},
        {"whatsapp": ["whatsapp::ready"]},
    ], trigger_snapshots=[[
        {"id": "whatsapp::event", "worker_name": "whatsapp", "namespace": "default"},
    ]])

    _, functions = collector.wait_for_worker("whatsapp", 20)

    assert names(functions) == ["whatsapp::ready"]
    assert clock["now"] == 5 * collector.SAMPLE_INTERVAL_SECONDS


def test_functions_arriving_during_final_sleep_cannot_publish_stale_trigger_only_interface(monkeypatch):
    clock = registration_sequence(monkeypatch, [{"whatsapp": []}], trigger_snapshots=[[
        {"id": "whatsapp::event", "worker_name": "whatsapp", "namespace": "default"},
    ]])
    engine_call = collector.run_iii

    def run_iii(function_id, payload):
        result = engine_call(function_id, payload)
        if function_id == "engine::functions::list" and clock["now"] >= 5:
            return {"functions": [{"function_id": "whatsapp::late", "worker_name": "whatsapp"}]}
        return result

    monkeypatch.setattr(collector, "run_iii", run_iii)

    with pytest.raises(TimeoutError, match="whatsapp.*stabilize"):
        collector.wait_for_worker("whatsapp", 6)

    assert clock["now"] == 6
