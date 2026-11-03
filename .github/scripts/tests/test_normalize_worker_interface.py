"""Tests for normalize_worker_interface (engine 0.17 worker_name shape)."""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from build_publish_payload import normalize_worker_interface  # noqa: E402


def test_collects_functions_by_worker_name_with_workers_baseline() -> None:
    baseline_workers = {
        "workers": [
            {"id": "configuration", "name": "configuration", "runtime": "engine"},
        ]
    }
    workers_json = {
        "workers": [
            *baseline_workers["workers"],
            {"id": "w1", "name": "harness", "runtime": "node"},
            {"id": "w2", "name": "turn-orchestrator", "runtime": "node"},
        ]
    }
    functions_json = {
        "functions": [
            {
                "function_id": "harness::send",
                "worker_name": "harness",
                "description": "kickoff",
            },
            {
                "function_id": "run::start",
                "worker_name": "turn-orchestrator",
                "description": "start run",
            },
            {
                "function_id": "configuration::get",
                "worker_name": "configuration",
                "description": "engine built-in",
            },
        ]
    }

    interface = normalize_worker_interface(
        worker_name="harness",
        workers_json=workers_json,
        functions_json=functions_json,
        baseline_workers_json=baseline_workers,
    )

    names = {fn["name"] for fn in interface["functions"]}
    assert names == {"harness::send", "run::start"}


def test_surfaces_typed_schemas_from_info_enriched_rows() -> None:
    """`engine::functions::list` rows carry no schemas — only
    `engine::functions::info` does, under `request_schema`/`response_schema`.
    Once the collector enriches each row from `::info`, normalize must surface
    those typed schemas (not the empty `{}` it produced when reading the
    never-present `request_format` key)."""
    workers_json = {
        "workers": [{"id": "llm-router", "name": "llm-router", "runtime": "rust"}],
    }
    req = {
        "type": "object",
        "title": "AbortRequest",
        "properties": {"request_id": {"type": "string"}},
    }
    resp = {
        "type": "object",
        "title": "AbortResponse",
        "properties": {"aborted": {"type": "boolean"}},
    }
    functions_json = {
        "functions": [
            {
                "function_id": "router::abort",
                "worker_name": "llm-router",
                "description": "abort",
                "request_schema": req,
                "response_schema": resp,
            },
        ]
    }

    interface = normalize_worker_interface(
        worker_name="llm-router",
        workers_json=workers_json,
        functions_json=functions_json,
    )

    assert interface["functions"] == [
        {
            "name": "router::abort",
            "description": "abort",
            "request_schema": req,
            "response_schema": resp,
            "args_format": None,
            "private": False,
            "observable": True,
            "metadata": {},
        }
    ]


def test_surfaces_an_event_source_observe_fields() -> None:
    """An event source is a private function whose handler returns the event:
    `request_schema` and `response_schema` describe the event, and
    `args_format` (from `engine::functions::list|info`) describes what its
    observers pass. The engine leaves `private: false` and `observable: true`
    out of its rows; the interface states both."""
    event = {
        "type": "object",
        "title": "SessionCreatedEvent",
        "properties": {"session_id": {"type": "string"}},
    }
    args_format = {
        "type": "object",
        "properties": {"expression": {"type": "string"}},
        "required": ["expression"],
    }
    interface = normalize_worker_interface(
        worker_name="session-manager",
        workers_json={
            "workers": [{"id": "session-manager", "name": "session-manager", "runtime": "rust"}]
        },
        functions_json={
            "functions": [
                {
                    "function_id": "session::on-created",
                    "worker_name": "session-manager",
                    "description": "A new session exists.",
                    "request_schema": event,
                    "response_schema": event,
                    "private": True,
                    "observer_count": 2,
                },
                {
                    "function_id": "session::on-tick",
                    "worker_name": "session-manager",
                    "description": "A tick.",
                    "request_schema": event,
                    "response_schema": event,
                    "args_format": args_format,
                    "private": True,
                    "observer_count": 0,
                },
                {
                    "function_id": "session::resolve",
                    "worker_name": "session-manager",
                    "description": "Carries a credential.",
                    "observable": False,
                    "observer_count": 0,
                },
            ]
        },
    )

    assert interface["functions"] == [
        {
            "name": "session::on-created",
            "description": "A new session exists.",
            "request_schema": event,
            "response_schema": event,
            "args_format": None,
            "private": True,
            "observable": True,
            "metadata": {},
        },
        {
            "name": "session::on-tick",
            "description": "A tick.",
            "request_schema": event,
            "response_schema": event,
            "args_format": args_format,
            "private": True,
            "observable": True,
            "metadata": {},
        },
        {
            "name": "session::resolve",
            "description": "Carries a credential.",
            "request_schema": {},
            "response_schema": {},
            "args_format": None,
            "private": False,
            "observable": False,
            "metadata": {},
        },
    ]


def test_collects_single_worker_without_baseline() -> None:
    workers_json = {
        "workers": [{"id": "shell", "name": "shell", "runtime": "rust"}],
    }
    functions_json = {
        "functions": [
            {
                "function_id": "shell::exec",
                "worker_name": "shell",
                "description": "run command",
            },
        ]
    }

    interface = normalize_worker_interface(
        worker_name="shell",
        workers_json=workers_json,
        functions_json=functions_json,
    )

    assert interface["functions"] == [
        {
            "name": "shell::exec",
            "description": "run command",
            "request_schema": {},
            "response_schema": {},
            "args_format": None,
            "private": False,
            "observable": True,
            "metadata": {},
        }
    ]
