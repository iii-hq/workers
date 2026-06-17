"""Tests for enrich_functions_with_schemas in collect_worker_interface.py.

`engine::functions::list` returns only `{function_id, worker_name, description}`
(a `FunctionSummary`); the typed request/response schemas live solely on
`engine::functions::info` (a `FunctionDetail`). The collector must fetch `::info`
per target function and merge the schemas into the list rows before normalizing,
or every function's schema collapses to the empty `{}` the publish-time
`--assert-typed-schemas` check rejects.
"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from collect_worker_interface import enrich_functions_with_schemas  # noqa: E402

REQ = {"type": "object", "properties": {"request_id": {"type": "string"}}}
RESP = {"type": "object", "properties": {"aborted": {"type": "boolean"}}}


def _details(mapping):
    """Return a fetch_detail stub backed by a {function_id: detail} mapping."""
    return lambda function_id: mapping.get(function_id)


def test_merges_info_schemas_into_target_rows() -> None:
    functions = [
        {"function_id": "router::abort", "worker_name": "llm-router", "description": "abort"},
    ]
    details = {
        "router::abort": {
            "function_id": "router::abort",
            "worker_name": "llm-router",
            "description": "abort",
            "request_schema": REQ,
            "response_schema": RESP,
            "metadata": {"registry_name": "abort"},
        }
    }

    enrich_functions_with_schemas(
        functions, ["router::abort"], fetch_detail=_details(details)
    )

    abort = functions[0]
    assert abort["request_schema"] == REQ
    assert abort["response_schema"] == RESP
    assert abort["metadata"] == {"registry_name": "abort"}


def test_leaves_untargeted_rows_untouched() -> None:
    functions = [
        {"function_id": "router::abort", "worker_name": "llm-router", "description": "abort"},
        {"function_id": "engine::log", "worker_name": "engine", "description": "log"},
    ]
    details = {
        "router::abort": {"request_schema": REQ, "response_schema": RESP},
        "engine::log": {"request_schema": REQ, "response_schema": RESP},
    }

    enrich_functions_with_schemas(
        functions, ["router::abort"], fetch_detail=_details(details)
    )

    assert "request_schema" in functions[0]
    assert "request_schema" not in functions[1]


def test_tolerates_missing_info_detail() -> None:
    """A `::info` lookup that returns None (call failed / unknown id) must not
    raise — the row keeps no schema and the typed-schema assertion flags it."""
    functions = [
        {"function_id": "router::abort", "worker_name": "llm-router", "description": "abort"},
    ]

    enrich_functions_with_schemas(
        functions, ["router::abort"], fetch_detail=lambda _id: None
    )

    assert "request_schema" not in functions[0]
