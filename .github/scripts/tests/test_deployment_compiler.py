import json
from pathlib import Path

import pytest

import build_skills_payload
import deployment_compiler
import deployment_targets


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


def test_compiler_derives_registry_interface_capture_policy_for_every_worker():
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


def test_kanban_rust_binary_builds_its_ui_with_the_workspace_pnpm_policy():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]

    descriptor = deployment_compiler.compile_worker(
        ROOT,
        "kanban",
        catalog["kanban"],
        "a" * 40,
        "b" * 64,
    )

    artifact = descriptor["artifact"]
    assert artifact["kind"] == "rust-binary"
    assert artifact["binary"] == "kanban"
    (frontend,) = artifact["frontends"]
    assert frontend["source_path"] == "kanban/ui"
    assert frontend["workspace_root"] == "."
    assert frontend["install_command"] == ["pnpm", "install", "--frozen-lockfile"]
    assert frontend["build_command"] == ["pnpm", "run", "build"]
    assert frontend["outputs"] == ["dist"]
    workspace = deployment_compiler.read_yaml(ROOT / frontend["workspace_root"] / "pnpm-workspace.yaml")
    assert "kanban/ui" in workspace["packages"]
    assert workspace["allowBuilds"] == {"esbuild": True, "protobufjs": False}


def test_registry_projection_carries_the_skills_payload():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]

    def projection(worker: str) -> dict:
        return deployment_compiler.compile_worker(ROOT, worker, catalog[worker], "a" * 40, "b" * 64)[
            "registry_projection"
        ]

    kanban = projection("kanban")["skills"]
    assert kanban == build_skills_payload.collect_skills(ROOT / "kanban")
    assert "SKILL.md" in kanban
    assert "skills/tickets/index.md" in kanban
    assert "agents/tech-lead.md" in kanban
    assert all(key.endswith(".md") and body.strip() for key, body in kanban.items())

    assert projection("acp")["skills"] == {}


def test_descriptor_schema_validates_the_skills_projection():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(
            encoding="utf-8"
        )
    )
    assert "skills" in schema["properties"]["registry_projection"]["required"]
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    validator = jsonschema.Draft202012Validator(schema)

    for worker in ("kanban", "acp"):
        validator.validate(deployment_compiler.compile_worker(ROOT, worker, catalog[worker], "a" * 40, "b" * 64))

    broken = deployment_compiler.compile_worker(ROOT, "kanban", catalog["kanban"], "a" * 40, "b" * 64)
    broken["registry_projection"]["skills"] = {"../escape.md": "x"}
    with pytest.raises(jsonschema.ValidationError):
        validator.validate(broken)


def test_binary_catalog_restores_intel_macos_except_the_microvm_sandbox():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    checked = set()
    for worker, value in catalog.items():
        if value.get("publish") is not True or value["artifact"]["kind"] != "rust-binary":
            continue
        descriptor = deployment_compiler.compile_worker(ROOT, worker, value, "a" * 40, "b" * 64)
        targets = descriptor["artifact"]["targets"]
        assert "aarch64-apple-darwin" in targets, worker
        assert ("x86_64-apple-darwin" in targets) == (worker != "sandbox-code-runner"), worker
        assert set(deployment_targets.normalize_targets(targets)) == set(targets), worker
        assert {unit["target"] for unit in descriptor["build_units"]} == set(targets), worker
        checked.add(worker)
    assert {"state", "http", "queue", "pubsub", "cron", "ide", "code-runner", "sandbox-code-runner"} <= checked


def test_rust_companions_are_validated_and_reach_the_schema():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(encoding="utf-8")
    )
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    compiled = deployment_compiler.compile_worker(ROOT, "judge-semif", catalog["judge-semif"], "a" * 40, "b" * 64)
    assert "libggml-vulkan.so" in compiled["artifact"]["companions"]
    jsonschema.Draft202012Validator(schema).validate(compiled)
    for broken in (["../escape.so"], ["lib/x.so"], [], [""], "libggml.so.0"):
        entry = json.loads(json.dumps(catalog["judge-semif"]))
        entry["artifact"]["companions"] = broken
        with pytest.raises(ValueError, match="companions"):
            deployment_compiler.compile_worker(ROOT, "judge-semif", entry, "a" * 40, "b" * 64)
