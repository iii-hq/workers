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
                "function_id": "harness::trigger",
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
    assert names == {"harness::trigger", "run::start"}


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
            "metadata": {},
        }
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
            "metadata": {},
        }
    ]
