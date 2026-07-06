"""The registered surface: exact function ids + typed schemas on every one."""

from __future__ import annotations

from src.handlers import create_handlers
from src.schemas import FUNCTIONS

EXPECTED_IDS = {
    "scrapling::fetch",
    "scrapling::stealthy-fetch",
    "scrapling::dynamic-fetch",
    "scrapling::screenshot",
    "scrapling::extract",
    "scrapling::css",
    "scrapling::xpath",
    "scrapling::regex",
    "scrapling::find-similar",
    "scrapling::find",
    "scrapling::find-by-text",
    "scrapling::find-by-regex",
    "scrapling::describe",
    "scrapling::to-markdown",
    "scrapling::session-open",
    "scrapling::session-fetch",
    "scrapling::session-close",
    "scrapling::session-list",
    "scrapling::crawl",
}


def test_function_ids_are_exactly_the_scrapling_surface():
    assert {f["id"] for f in FUNCTIONS} == EXPECTED_IDS


def test_every_function_has_a_registered_handler():
    handlers = create_handlers(lambda: {})
    for f in FUNCTIONS:
        assert f["handler"] in handlers, f"missing handler for {f['id']}"


def test_schemas_are_typed_objects_not_anyvalue():
    # The publish pipeline rejects untyped/AnyValue schemas; every request and
    # response must be a typed object schema with declared properties.
    for f in FUNCTIONS:
        for kind in ("request", "response"):
            schema = f[kind]
            assert schema.get("type") == "object", f"{f['id']} {kind} not an object"
            assert schema.get("properties"), f"{f['id']} {kind} has no properties"
