"""Tests for enrich_trigger_types_with_schemas in collect_worker_interface.py.

`engine::triggers::list` returns only `{id, worker_name, description}`
(a `TriggerTypeSummary`); the typed schemas live solely on
`engine::triggers::info` (a `TriggerTypeDetail`), under `configuration_schema`
(binding config) / `request_schema` (delivered payload) / `response_schema`.
The collector must fetch `::info` per publishable trigger type and merge the
schemas into the list rows before normalizing, or every trigger schema
collapses to the empty `{}` that renders as 'unknown' in the registry.
"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import collect_worker_interface  # noqa: E402
from collect_worker_interface import enrich_trigger_types_with_schemas  # noqa: E402

CFG = {"type": "object", "properties": {"session_id": {"type": "string"}}}
REQ = {"type": "object", "properties": {"entry_id": {"type": "string"}}}


def _details(mapping):
    """Return a fetch_detail stub backed by a {trigger_id: detail} mapping."""
    return lambda trigger_id, _namespace=None: mapping.get(trigger_id)


def test_fetches_info_in_the_namespace_reported_by_list(monkeypatch) -> None:
    calls = []

    def fake_run_iii(function_path, payload):
        calls.append((function_path, payload))
        return {"id": "browser::console-event", "namespace": "api-ref-repair"}

    monkeypatch.setattr(collect_worker_interface, "run_iii", fake_run_iii)

    detail = collect_worker_interface._fetch_trigger_detail(
        "browser::console-event", "api-ref-repair"
    )

    assert detail == {
        "id": "browser::console-event",
        "namespace": "api-ref-repair",
    }
    assert calls == [
        (
            "engine::triggers::info",
            {"id": "browser::console-event", "namespace": "api-ref-repair"},
        )
    ]


def test_fetches_info_without_namespace_for_legacy_list_rows(monkeypatch) -> None:
    calls = []

    def fake_run_iii(function_path, payload):
        calls.append((function_path, payload))
        return {"id": "session::created"}

    monkeypatch.setattr(collect_worker_interface, "run_iii", fake_run_iii)

    collect_worker_interface._fetch_trigger_detail("session::created")

    assert calls == [("engine::triggers::info", {"id": "session::created"})]


def test_merges_info_schemas_into_target_rows() -> None:
    triggers = [
        {
            "id": "session::created",
            "worker_name": "session-manager",
            "description": "A new session exists.",
        },
    ]
    details = {
        "session::created": {
            "id": "session::created",
            "worker_name": "session-manager",
            "description": "A new session exists.",
            "configuration_schema": CFG,
            "request_schema": REQ,
            "instance_count": 0,
        }
    }

    enrich_trigger_types_with_schemas(
        triggers, ["session::created"], fetch_detail=_details(details)
    )

    created = triggers[0]
    assert created["configuration_schema"] == CFG
    assert created["request_schema"] == REQ


def test_leaves_untargeted_rows_untouched() -> None:
    triggers = [
        {"id": "session::created", "worker_name": "session-manager", "description": "x"},
        {"id": "engine::cron", "worker_name": "engine", "description": "builtin"},
    ]
    details = {
        "session::created": {"configuration_schema": CFG},
        "engine::cron": {"configuration_schema": CFG},
    }

    enrich_trigger_types_with_schemas(
        triggers, ["session::created"], fetch_detail=_details(details)
    )

    assert "configuration_schema" in triggers[0]
    assert "configuration_schema" not in triggers[1]


def test_tolerates_missing_info_detail() -> None:
    """A `::info` lookup that returns None (call failed / unknown id) must not
    raise — the row keeps no schema and publishes the empty ('unknown') schema."""
    triggers = [
        {"id": "session::created", "worker_name": "session-manager", "description": "x"},
    ]

    enrich_trigger_types_with_schemas(
        triggers, ["session::created"], fetch_detail=lambda _id, _namespace: None
    )

    assert "configuration_schema" not in triggers[0]


def test_enrichment_passes_each_list_rows_namespace_to_info() -> None:
    triggers = [
        {
            "id": "browser::console-event",
            "namespace": "api-ref-repair",
            "worker_name": "browser",
            "description": "x",
        },
        {
            "id": "browser::console-event",
            "worker_name": "browser",
            "description": "legacy default row",
        },
    ]
    calls = []

    def fetch_detail(trigger_id, namespace):
        calls.append((trigger_id, namespace))
        return {"configuration_schema": CFG}

    enrich_trigger_types_with_schemas(
        triggers, ["browser::console-event"], fetch_detail=fetch_detail
    )

    assert calls == [
        ("browser::console-event", "api-ref-repair"),
        ("browser::console-event", None),
    ]
    assert triggers[0]["configuration_schema"] == CFG
    assert triggers[1]["configuration_schema"] == CFG
