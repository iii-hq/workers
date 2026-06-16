"""Tests for the --assert-non-empty / --assert-typed-schemas flags on
collect_worker_interface.py."""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "collect_worker_interface.py"

TYPED_SCHEMA = {"type": "object", "properties": {"x": {"type": "string"}}}
# What schemars emits for an untyped `serde_json::Value` handler.
ANYVALUE_SCHEMA = {"$schema": "http://json-schema.org/draft-07/schema#", "title": "AnyValue"}


def write_payload(path: Path, *, functions: list) -> None:
    path.write_text(json.dumps({"functions": functions, "triggers": []}))


def typed_fn(name: str = "smoke::ping") -> dict:
    return {"name": name, "request_schema": TYPED_SCHEMA, "response_schema": TYPED_SCHEMA}


class TestAssertNonEmpty:
    def test_passes_when_functions_non_empty(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[{"id": "smoke::ping"}])
        r = subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-non-empty", "--assert-file", str(out)],
            capture_output=True, text=True,
        )
        assert r.returncode == 0, r.stderr

    def test_fails_when_functions_empty(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[])
        r = subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-non-empty", "--assert-file", str(out)],
            capture_output=True, text=True,
        )
        assert r.returncode != 0
        assert "empty" in (r.stderr + r.stdout).lower()

    def test_fails_when_functions_key_missing(self, tmp_path):
        out = tmp_path / "interface.json"
        out.write_text("{}")
        r = subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-non-empty", "--assert-file", str(out)],
            capture_output=True, text=True,
        )
        assert r.returncode != 0


class TestAssertTypedSchemas:
    def _run(self, out: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-typed-schemas", "--assert-file", str(out)],
            capture_output=True, text=True,
        )

    def test_passes_when_all_schemas_typed(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[typed_fn("a::b"), typed_fn("c::d")])
        r = self._run(out)
        assert r.returncode == 0, r.stderr

    def test_fails_on_anyvalue_request_schema(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[{
            "name": "router::chat",
            "request_schema": ANYVALUE_SCHEMA,
            "response_schema": TYPED_SCHEMA,
        }])
        r = self._run(out)
        assert r.returncode != 0
        assert "router::chat.request_schema" in (r.stderr + r.stdout)

    def test_fails_on_empty_response_schema(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[{
            "name": "router::abort",
            "request_schema": TYPED_SCHEMA,
            "response_schema": {},
        }])
        r = self._run(out)
        assert r.returncode != 0
        assert "router::abort.response_schema" in (r.stderr + r.stdout)

    def test_accepts_nullable_anyof_response(self, tmp_path):
        # `Option<T>` responses serialize to an `anyOf` schema — still typed.
        out = tmp_path / "interface.json"
        write_payload(out, functions=[{
            "name": "router::models::get",
            "request_schema": TYPED_SCHEMA,
            "response_schema": {"anyOf": [{"$ref": "#/definitions/X"}, {"type": "null"}]},
        }])
        r = self._run(out)
        assert r.returncode == 0, r.stderr
