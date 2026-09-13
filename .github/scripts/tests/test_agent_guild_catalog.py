"""The optional worker must be buildable, discoverable and capture its real interface."""
import json
from pathlib import Path

import yaml
from deployment_compiler import compile_worker

ROOT = Path(__file__).resolve().parents[3]


def test_agent_guild_catalog_descriptor():
    private = yaml.safe_load((ROOT / ".deploy/workers.yaml").read_text())["workers"]["agent-guild"]
    assert set(private) == {"source", "artifact", "publish"}
    descriptor = compile_worker(ROOT, "agent-guild", private, "a" * 40, "b" * 64)
    assert descriptor["interface_capture"] == "required"
    assert descriptor["publish"] is True
    assert descriptor["registry_projection"]["config"] == {}
    assert private["artifact"]["include"] == ["dist/bundle/index.mjs", "iii.worker.yaml"]
    package = json.loads((ROOT / "agent-guild/package.json").read_text())
    assert package["dependencies"]["iii-sdk"] == "0.23.0"
    assert package["scripts"]["test:e2e"] == "node tests/e2e/run.mjs"


def test_agent_guild_protocol_workflow_keeps_native_gates():
    workflow = yaml.safe_load((ROOT / ".github/workflows/agent-guild-e2e.yml").read_text())
    steps = workflow["jobs"]["e2e"]["steps"]
    commands = "\n".join(step.get("run", "") for step in steps)
    assert "--ignore-scripts" in commands
    assert "test:e2e" in commands
    assert "validate_worker.py" in commands
    assert "deployment_compiler.py compile-index" in commands
    assert "sha256sum --check" in commands
    assert "iii/v0.23.0" in commands
    source = (ROOT / "agent-guild/tests/e2e/run.mjs").read_text()
    assert "collect_worker_interface.py" in source
    assert "--assert-typed-schemas" in source
    assert "registerWorker" in source
    assert "createServer" in source
