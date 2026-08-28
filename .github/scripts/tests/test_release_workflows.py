from __future__ import annotations

from pathlib import Path

import yaml


ROOT = Path(__file__).parents[2]
WORKFLOWS = ROOT / "workflows"
COMMON = {"operation_id", "step_id"}

EXPECTED_INPUTS = {
    "harness-e2e-shadow.yml": {"campaign_id", "execution_id", "attempt", "execution_contract"},
    "prepare-release.yml": COMMON
    | {
        "release_intent_id",
        "candidate_id",
        "release_attempt_id",
        "worker",
        "stable_version",
        "candidate_version",
        "source_sha",
        "targets",
        "plan_hash",
        "dispatch_nonce",
    },
    "publish-candidate.yml": COMMON
    | {
        "release_intent_id",
        "candidate_id",
        "release_attempt_id",
        "worker",
        "stable_version",
        "candidate_version",
        "source_sha",
        "targets",
        "prepared_sha",
        "prepared_run_id",
        "prepared_artifact",
        "plan_hash",
        "expected_next_version",
        "dispatch_nonce",
    },
    "publish-stable.yml": COMMON
    | {
        "release_intent_id",
        "candidate_id",
        "release_attempt_id",
        "worker",
        "candidate_version",
        "stable_version",
        "source_operation_id",
        "source_sha",
        "targets",
        "plan_hash",
        "expected_next_version",
        "expected_latest_version",
        "recovery_run_id",
        "recovery_operation_id",
        "recovery_step_id",
        "dispatch_nonce",
    },
    "finalize-registry.yml": COMMON
    | {
        "release_intent_id",
        "candidate_id",
        "release_attempt_id",
        "worker",
        "candidate_version",
        "stable_version",
        "source_sha",
        "plan_hash",
        "expected_next_version",
        "expected_latest_version",
        "dispatch_nonce",
    },
    "create-tag.yml": COMMON | {"worker", "target_version", "registry_tag", "experimental", "expected_current_version", "source_sha"},
    "create-prerelease-tag.yml": COMMON | {"worker", "target_version", "source_sha", "experimental"},
    "release.yml": COMMON | {"source_tag_step_id", "tag", "publish_registry"},
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
    "prepare-release.yml",
    "publish-candidate.yml",
    "publish-stable.yml",
    "finalize-registry.yml",
    "create-tag.yml",
    "create-prerelease-tag.yml",
    "release.yml",
    "container-alias.yml",
    "promote-registry.yml",
    "reconcile-github-release.yml",
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
        optional_defaults = {
            "publish-stable.yml": {
                "recovery_run_id": "0",
                "recovery_operation_id": "none",
                "recovery_step_id": "none",
            },
        }
        for input_name, definition in inputs.items():
            if input_name in optional_defaults.get(name, {}):
                assert definition.get("required") == "false", (name, input_name)
                assert definition.get("default") == optional_defaults[name][input_name], (name, input_name)
            else:
                assert definition.get("required") == "true", (name, input_name)


def test_every_dispatch_is_actor_gated_and_emits_factual_evidence() -> None:
    for name in EXPECTED_INPUTS:
        body = (WORKFLOWS / name).read_text()
        assert "release_control_contract.py validate-dispatch" in body, name
        assert "RELEASE_CONTROL_BOT_LOGIN" in body, name
        if name == "harness-e2e-shadow.yml":
            assert "e2e-observation-${{ inputs.campaign_id }}-${{ inputs.execution_id }}" in body, name
            assert "ACTIONS_ID_TOKEN_REQUEST_TOKEN" in body, name
        else:
            assert "execution-result-${{ inputs.operation_id }}-${{ inputs.step_id }}" in body, name
        assert ("--mutating" in body) == (name in MUTATING), name


def test_harness_shadow_uses_the_general_self_hosted_pool() -> None:
    observe = workflow(WORKFLOWS / "harness-e2e-shadow.yml")["jobs"]["observe"]
    assert observe["runs-on"] == ["self-hosted", "Linux", "X64", "general"]
    assert observe["environment"] == "harness-e2e-trusted"


def test_candidate_smoke_prepares_kvm_only_for_scrapling() -> None:
    steps = workflow(WORKFLOWS / "candidate-smoke.yml")["jobs"]["smoke"]["steps"]
    prepare = next(step for step in steps if step.get("name") == "Prepare KVM for Scrapling sandbox")
    assert prepare["if"] == "inputs.worker == 'scrapling'"
    assert "test -c /dev/kvm" in prepare["run"]
    assert "sudo chmod 0666 /dev/kvm" in prepare["run"]


def test_candidate_publish_retries_and_safely_joins_prepared_commits() -> None:
    path = WORKFLOWS / "publish-candidate.yml"
    value = workflow(path)
    publish = value["jobs"]["publish"]
    assert "concurrency" not in publish

    steps = publish["steps"]
    verify = next(step for step in steps if step.get("name") == "Verify prepared artifact and release metadata")
    push = next(step for step in steps if step.get("name") == "Push candidate commit and annotated tag atomically")

    assert 'git merge-base --is-ancestor "$SOURCE_SHA" "$PREPARED_SHA"' in verify["run"]
    assert 'git diff --quiet "$SOURCE_SHA" "$main_sha" -- "$WORKER"' in verify["run"]
    assert 'case "$changed" in' in verify["run"]
    assert '"$WORKER"/*)' in verify["run"]
    assert "prepare_main_target()" in push["run"]
    assert "publish_git_refs()" in push["run"]
    assert 'git merge --no-ff --no-edit "$PREPARED_SHA"' in push["run"]
    assert "for attempt in $(seq 1 60)" in push["run"]
    assert "main advanced during candidate publication; retrying CAS" in push["run"]
    assert 'git push --atomic origin' in push["run"]


def test_reusable_harness_executor_has_no_implicit_inputs() -> None:
    inputs = workflow(WORKFLOWS / "_harness-e2e.yml")["on"]["workflow_call"]["inputs"]
    assert inputs
    assert all(definition.get("required") == "true" for definition in inputs.values())
    assert all("default" not in definition for definition in inputs.values())


def test_reusable_release_executors_have_no_implicit_inputs() -> None:
    for name in RELEASE_EXECUTORS:
        inputs = workflow(WORKFLOWS / name)["on"]["workflow_call"]["inputs"]
        assert inputs, name
        optional_defaults = {
            "_publish-registry.yml": {"expected_current_version": ""},
            "publish-stable.yml": {
                "recovery_run_id": "0",
                "recovery_operation_id": "none",
                "recovery_step_id": "none",
            },
        }
        for input_name, definition in inputs.items():
            if input_name in optional_defaults.get(name, {}):
                assert definition.get("required") == "false", (name, input_name)
                assert definition.get("default") == optional_defaults[name][input_name], (name, input_name)
            else:
                assert definition.get("required") == "true", (name, input_name)
                assert "default" not in definition, (name, input_name)


def test_registry_publish_authenticates_iii_installer() -> None:
    body = (WORKFLOWS / "_publish-registry.yml").read_text()
    assert "GITHUB_TOKEN: ${{ github.token }}" in body


def test_release_target_policy_excludes_windows_and_requires_complete_assets() -> None:
    body = (WORKFLOWS / "_rust-binary.yml").read_text()
    assert "windows-latest" not in body
    assert "x86_64-pc-windows-msvc" not in body

    resolver = (WORKFLOWS.parent / "scripts" / "resolve_binary_artifacts.py").read_text()
    assert "missing binary artefacts for targets" in resolver


def test_prepare_release_expands_the_computed_target_list_with_matrix_include() -> None:
    jobs = workflow(WORKFLOWS / "prepare-release.yml")["jobs"]
    strategy = jobs["build"]["strategy"]

    assert strategy["fail-fast"] == "false"
    assert strategy["matrix"] == {
        "include": "${{ fromJSON(needs.matrix.outputs.include) }}",
    }


def test_publish_stable_expands_the_computed_target_list_with_matrix_include() -> None:
    jobs = workflow(WORKFLOWS / "publish-stable.yml")["jobs"]
    strategy = jobs["build"]["strategy"]

    assert strategy["fail-fast"] == "false"
    assert strategy["matrix"] == {
        "include": "${{ fromJSON(needs.matrix.outputs.include) }}",
    }


def test_registry_publish_keeps_stdio_workers_alive_for_interface_collection() -> None:
    body = (WORKFLOWS / "_publish-registry.yml").read_text()
    assert "start_worker_with_open_stdin" in body
    assert "stdin=subprocess.PIPE" in body
    assert "signal.signal(signal.SIGTERM" in body


def test_registry_publication_builds_every_payload_before_commit_last_writes() -> None:
    path = WORKFLOWS / "_publish-registry.yml"
    body = path.read_text()
    steps = workflow(path)["jobs"]["publish"]["steps"]
    names = [step.get("name") for step in steps]

    build_payload = names.index("Build payload")
    build_skills = names.index("Build skills payload")
    publish_version = names.index("Publish and verify immutable Registry version")
    publish_skills = names.index("Publish and verify exact Registry skills")
    commit_channel = names.index("Commit and verify Registry channel with exact CAS")

    assert build_payload < publish_version
    assert build_skills < publish_version < publish_skills < commit_channel
    assert steps[commit_channel]["if"] == "inputs.registry_tag != 'none'"
    assert body.count("registry_publication.py") == 3
    assert '-X POST "$API_URL/publish"' not in body
    assert '-X POST "$API_URL/w/$WORKER/skills"' not in body
    assert '-X PUT "$API_URL/w/$WORKER/tags/$REGISTRY_TAG"' not in body


def test_rust_binary_cache_is_keyed_by_frontend_bundle_digest() -> None:
    jobs = workflow(WORKFLOWS / "_rust-binary.yml")["jobs"]
    web_build = jobs["web-build"]
    assert web_build["outputs"]["frontend_digest"] == "${{ steps.stage.outputs.frontend_digest }}"
    assert web_build["outputs"]["frontends"] == "${{ steps.dirs.outputs.frontends }}"

    stage = next(step for step in web_build["steps"] if step.get("name") == "Stage frontend bundles")
    assert stage["id"] == "stage"
    assert "frontend_digest=$digest" in stage["run"]
    assert "git hash-object" in stage["run"]

    build_steps = jobs["build"]["steps"]
    cache = next(step for step in build_steps if step.get("uses") == "Swatinem/rust-cache@v2")
    assert "needs.web-build.outputs.frontend_digest" in cache["with"]["key"]

    verify = next(step for step in build_steps if step.get("name") == "Verify frontend bundle digest")
    assert verify["if"] == "inputs.web_bundle"
    assert verify["env"]["EXPECTED_DIGEST"] == "${{ needs.web-build.outputs.frontend_digest }}"
    assert "git hash-object" in verify["run"]
    assert '[[ "$actual" == "$EXPECTED_DIGEST" ]]' in verify["run"]


def test_release_detects_frontends_from_path_dependencies() -> None:
    steps = workflow(WORKFLOWS / "release.yml")["jobs"]["setup"]["steps"]
    detect = next(step for step in steps if step.get("name") == "Detect web bundle")

    assert detect["env"]["MANIFEST"] == "${{ steps.meta.outputs.manifest }}"
    assert "manifest_version.py frontend-bundles" in detect["run"]
