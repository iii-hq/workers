import json
from pathlib import Path

import pytest

import deployment_compiler


ROOT = Path(__file__).resolve().parents[3]


def test_canonical_numbers_match_json_stringify_representation():
    assert deployment_compiler.canonical_bytes(
        {
            "analysis": {"max_cost_usd": 2.0, "max_turns": 4, "ratio": 0.5},
            "negative_zero": -0.0,
        }
    ) == (
        b'{"analysis":{"max_cost_usd":2,"max_turns":4,"ratio":0.5},'
        b'"negative_zero":0}'
    )


def test_canonical_numbers_reject_non_json_values():
    with pytest.raises(ValueError):
        deployment_compiler.canonical_bytes({"budget": float("nan")})


def test_compiler_derives_explicit_interface_capture_policy_for_every_worker():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    descriptors = {
        worker: deployment_compiler.compile_worker(
            ROOT,
            worker,
            value,
            "a" * 40,
            "b" * 64,
        )
        for worker, value in catalog.items()
        if value.get("publish") is True
    }

    assert descriptors["acp"]["interface_capture"] == "skipped"
    assert descriptors["lsp"]["interface_capture"] == "skipped"
    assert {
        worker
        for worker, descriptor in descriptors.items()
        if descriptor["interface_capture"] != "required"
    } == {"acp", "lsp"}
    assert descriptors["database"]["runtime"]["interface_config"] == {
        "path": "config.collect.yaml",
        "sha256": deployment_compiler.file_sha256(ROOT / "database" / "config.collect.yaml"),
    }
    assert descriptors["web"]["runtime"]["interface_config"] is None


def test_descriptor_schema_requires_explicit_interface_capture_policy():
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(
            encoding="utf-8"
        )
    )

    assert "interface_capture" in schema["required"]
    assert schema["properties"]["interface_capture"] == {
        "enum": ["required", "skipped"]
    }
