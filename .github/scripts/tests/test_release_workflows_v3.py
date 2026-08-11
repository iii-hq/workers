from __future__ import annotations

from pathlib import Path

import yaml


ROOT = Path(__file__).parents[2]
WORKFLOWS = ROOT / "workflows"
COMMON = {"contract_version", "operation_id", "step_id"}

EXPECTED_INPUTS = {
    "create-tag.yml": COMMON | {"worker", "target_version", "registry_tag", "experimental", "expected_current_version", "source_sha"},
    "create-prerelease-tag.yml": COMMON | {"worker", "target_version", "source_sha", "experimental"},
    "release.yml": COMMON | {"source_tag_step_id", "tag", "publish_registry"},
    "create-lsp-vscode-tag.yml": COMMON | {"source_sha", "target_version"},
    "release-lsp-vscode.yml": COMMON | {"source_tag_step_id", "tag", "targets"},
    "candidate-smoke.yml": COMMON | {"tag", "worker", "version", "release_run_id", "release_run_attempt"},
    "container-alias.yml": COMMON | {"worker", "version", "channel", "expected_digest"},
    "promote-registry.yml": COMMON
    | {
        "worker",
        "version",
        "expected_next_version",
        "expected_latest_version",
        "release_run_id",
        "release_run_attempt",
        "candidate_evidence_run_id",
        "candidate_evidence_run_attempt",
        "e2e_run_id",
        "e2e_run_attempt",
    },
    "reconcile-github-release.yml": COMMON | {"worker", "version", "tag", "state"},
    "verify-release.yml": COMMON
    | {"worker", "version", "channel", "tag", "deploy", "expected_digest", "verify_registry"},
    "publish-worker-skills.yml": COMMON | {"worker", "version"},
    "harness-e2e-registry.yml": COMMON
    | {
        "source_sha",
        "lane",
        "channel",
        "release_worker",
        "release_version",
        "release_tag",
        "release_run_id",
        "release_run_attempt",
        "stack_versions",
        "validation_profile",
        "scenarios_json",
        "required_scenarios_json",
        "policy_digest",
        "policy_version",
        "subjects",
        "judge_model",
        "judge_provider",
        "runs",
    },
    "harness-e2e-source.yml": COMMON
    | {
        "source_sha",
        "scenarios_json",
        "required_scenarios_json",
        "policy_digest",
        "policy_version",
        "subjects",
        "judge_model",
        "judge_provider",
        "runs",
    },
    "harness-quickstart.yml": COMMON
    | {
        "source_sha",
        "cli_channel",
        "registry_tag",
        "release_worker",
        "release_version",
        "release_tag",
        "release_run_id",
        "release_run_attempt",
    },
    "database-e2e.yml": COMMON | {"source_sha"},
    "rbac-proxy-e2e.yml": COMMON | {"source_sha"},
    "shell-e2e.yml": COMMON | {"source_sha"},
    "storage-e2e.yml": COMMON | {"source_sha"},
}

MUTATING = {
    "create-tag.yml",
    "create-prerelease-tag.yml",
    "release.yml",
    "create-lsp-vscode-tag.yml",
    "release-lsp-vscode.yml",
    "container-alias.yml",
    "promote-registry.yml",
    "reconcile-github-release.yml",
    "publish-worker-skills.yml",
}

RELEASE_EXECUTORS = {
    "_bundle.yml",
    "_container.yml",
    "_publish-registry.yml",
    "_rust-binary.yml",
}


def workflow(path: Path) -> dict:
    value = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
    assert isinstance(value, dict)
    return value


def test_release_control_workflow_input_contract_is_exact() -> None:
    dispatch_files = set()
    for path in WORKFLOWS.glob("*.yml"):
        value = workflow(path)
        triggers = value.get("on")
        if isinstance(triggers, dict) and "workflow_dispatch" in triggers:
            dispatch_files.add(path.name)
    assert dispatch_files == set(EXPECTED_INPUTS)
    for name, expected in EXPECTED_INPUTS.items():
        dispatch = workflow(WORKFLOWS / name)["on"]["workflow_dispatch"]
        inputs = dispatch.get("inputs", {}) if isinstance(dispatch, dict) else {}
        assert set(inputs) == expected, name
        assert all(input_definition.get("required") == "true" for input_definition in inputs.values()), name


def test_every_dispatch_is_actor_gated_and_emits_factual_evidence() -> None:
    for name in EXPECTED_INPUTS:
        body = (WORKFLOWS / name).read_text()
        assert "release_control_contract.py validate-dispatch" in body, name
        assert "RELEASE_CONTROL_BOT_LOGIN" in body, name
        assert "execution-result-${{ inputs.operation_id }}-${{ inputs.step_id }}" in body, name
        assert ("--mutating" in body) == (name in MUTATING), name


def test_reusable_harness_executor_has_no_implicit_inputs() -> None:
    inputs = workflow(WORKFLOWS / "_harness-e2e.yml")["on"]["workflow_call"]["inputs"]
    assert inputs
    assert all(definition.get("required") == "true" for definition in inputs.values())
    assert all("default" not in definition for definition in inputs.values())


def test_reusable_release_executors_have_no_implicit_inputs() -> None:
    for name in RELEASE_EXECUTORS:
        inputs = workflow(WORKFLOWS / name)["on"]["workflow_call"]["inputs"]
        assert inputs, name
        assert all(definition.get("required") == "true" for definition in inputs.values()), name
        assert all("default" not in definition for definition in inputs.values()), name
