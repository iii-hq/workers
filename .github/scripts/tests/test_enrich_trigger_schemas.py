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

from collect_worker_interface import enrich_trigger_types_with_schemas  # noqa: E402

CFG = {"type": "object", "properties": {"session_id": {"type": "string"}}}
REQ = {"type": "object", "properties": {"entry_id": {"type": "string"}}}


def _details(mapping):
    """Return a fetch_detail stub backed by a {trigger_id: detail} mapping."""
    return lambda trigger_id: mapping.get(trigger_id)


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
        triggers, ["session::created"], fetch_detail=lambda _id: None
    )

    assert "configuration_schema" not in triggers[0]
