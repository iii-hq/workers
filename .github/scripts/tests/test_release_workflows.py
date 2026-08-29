import re
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / ".github" / "workflows"
ENTRYPOINTS = {
    "release-prepare.yml": "prepare",
    "release-candidate-publish.yml": "candidate_publish",
    "release-candidate-smoke.yml": "candidate_smoke",
    "release-stable-publish.yml": "stable_publish",
    "release-image-alias.yml": "image_alias",
    "release-finalize.yml": "finalize",
    "release-verify.yml": "verify",
}
REUSABLE = {"_release-build.yml", "_release-registry.yml", "_worker-e2e.yml"}


def body(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def test_exact_release_topology_and_no_legacy_wrappers():
    for name in set(ENTRYPOINTS) | REUSABLE:
        assert (WORKFLOWS / name).is_file(), name
    old = {"prepare-release.yml", "publish-candidate.yml", "candidate-smoke.yml", "publish-stable.yml",
           "container-alias.yml", "finalize-registry.yml", "verify-release.yml", "release.yml",
           "_rust-binary.yml", "_publish-registry.yml", "_harness-e2e.yml", "_bundle.yml", "_container.yml"}
    assert not {path.name for path in WORKFLOWS.glob("*.yml")}.intersection(old)


def test_every_entrypoint_authorizes_and_posts_nominal_release_result():
    for name, phase in ENTRYPOINTS.items():
        text = body(name)
        assert "authorize-dispatch" in text, name
        assert "post-result" in text, name
        assert "--output release-result.json" in text, name
        assert f"--phase {phase}" in text, name
        assert "release-result-${{ env.CANDIDATE_ID }}-${{ env.STEP_ID }}-attempt-${{ github.run_attempt }}" in text
        assert f"--workflow '{name}'" in text
        assert "${{ fromJSON(inputs.identity).operation_id }} · ${{ fromJSON(inputs.identity).step_id }} · ${{ fromJSON(inputs.identity).dispatch_nonce }}" in text
        assert "validate-identity --identity \"$RELEASE_IDENTITY\"" in text


def test_every_entrypoint_reports_and_uploads_result_even_after_effect_failure():
    for name in ENTRYPOINTS:
        workflow = yaml.safe_load(body(name))
        result_steps = [
            step
            for job in workflow["jobs"].values()
            for step in job.get("steps", [])
            if "release-result.json" in str(step)
            and (
                "write-result" in step.get("run", "")
                or "post-result" in step.get("run", "")
                or str(step.get("uses", "")).startswith("actions/upload-artifact@")
            )
        ]
        assert len(result_steps) == 3, name
        assert all(step.get("if") == "always()" for step in result_steps), name


def test_dispatch_inputs_use_one_identity_object_and_fit_github_limit():
    legacy_identity_inputs = {
        "operation_id", "step_id", "release_intent_id", "candidate_id",
        "release_attempt_id", "dispatch_nonce", "plan_hash",
    }
    for name in ENTRYPOINTS:
        workflow = yaml.safe_load(body(name))
        inputs = workflow[True]["workflow_dispatch"]["inputs"]
        assert len(inputs) <= 10, name
        assert inputs["identity"] == {"required": True, "type": "string"}
        assert legacy_identity_inputs.isdisjoint(inputs), name
        assert "prepared_artifact" not in inputs, name


def test_dispatch_values_are_never_rendered_into_shell_source():
    for name in set(ENTRYPOINTS) | {"_release-build.yml", "_release-registry.yml"}:
        workflow = yaml.safe_load(body(name))
        for job in workflow["jobs"].values():
            for step in job.get("steps", []):
                command = step.get("run", "")
                assert "${{ inputs." not in command, f"{name}: {step.get('name', 'unnamed')}"
                assert "fromJSON(inputs.identity)" not in command, f"{name}: {step.get('name', 'unnamed')}"


def test_new_release_workflows_pin_third_party_actions_by_sha():
    for name in set(ENTRYPOINTS) | REUSABLE:
        for line in body(name).splitlines():
            if "uses:" not in line or "./.github/" in line:
                continue
            reference = line.split("uses:", 1)[1].strip().split()[0]
            assert re.search(r"@[0-9a-f]{40}$", reference), f"{name}: mutable action {reference}"


def test_post_prepare_workflows_consume_only_descriptor_and_prepared_artifacts():
    for name in set(ENTRYPOINTS) - {"release-prepare.yml"}:
        text = body(name)
        if name != "release-candidate-smoke.yml":
            assert "worker-compose.yaml" not in text, name
        assert "package_manifest" not in text, name
        assert "iii compose descriptor" not in text, name


def test_release_train_never_reads_public_worker_manifests():
    release_files = [WORKFLOWS / name for name in set(ENTRYPOINTS) | REUSABLE]
    release_files.append(WORKFLOWS / "release-descriptor-index.yml")
    release_files.extend(
        path for path in (ROOT / ".github" / "scripts").glob("release_*.py")
        if path.name != "release_compiler.py"
    )
    for path in release_files:
        assert "iii.worker.yaml" not in path.read_text(encoding="utf-8"), path


def test_release_prepare_fans_one_job_per_compiled_build_unit():
    text = body("release-prepare.yml")
    assert "descriptor_run_id" in text
    assert "descriptor_artifact" in text
    assert "release_train.py select-descriptor" in text
    assert "iii compose descriptor" not in text
    assert "Build pinned iii compiler" not in text
    assert "frontend-bundles" not in text
    assert "release_train.py frontend-metadata" in text
    assert "release_train.py build-frontends" in text
    assert "release_train.py matrix" in text
    assert "matrix: ${{ fromJSON(needs.prepare.outputs.matrix) }}" in text
    assert "unit: ${{ matrix.unit }}" in text


def test_prepare_runs_adapter_from_prepared_bytes_and_records_interface():
    text = body("release-prepare.yml")
    workflow = yaml.safe_load(text)
    assert "adapter" in workflow["jobs"]
    assert workflow["jobs"]["adapter"]["needs"] == ["prepare", "assemble"]
    assert "release_interface.py stage" in text
    assert "release_interface.py run-adapter" in text
    assert "release_interface.py snapshot" in text
    assert "release_interface.py build-evidence" in text
    assert "release-prepared-${{ env.CANDIDATE_ID }}" in text
    adapter_text = str(workflow["jobs"]["adapter"])
    assert "inputs.source_sha" not in adapter_text
    assert "worker-compose.yaml" not in adapter_text


def test_candidate_smoke_boots_registry_candidate_and_compares_prepared_interface():
    text = body("release-candidate-smoke.yml")
    assert '"worker": f"package://{worker}", "version": "next"' in text
    assert "iii compose --engine \"$engine_url\"" in text
    assert "collect_worker_interface.py" in text
    assert "release_interface.py compare" in text
    assert "release_interface.py verify-evidence" in text
    assert ".registry_projection.type" in text
    assert ".registry_projection.dependencies" in text
    assert "EXPECTED_IMAGE_DIGEST" in text
    assert ".image' smoke-resolution.json" in text
    assert "iii worker add" not in text
    assert "ref: ${{ inputs.source_sha }}" not in text
    assert "ref: ${{ inputs.prepared_sha }}" not in text


def test_all_post_prepare_phases_verify_final_evidence_and_report_it():
    for name in set(ENTRYPOINTS) - {"release-prepare.yml"}:
        text = body(name)
        assert "release_interface.py verify-evidence" in text, name
        assert "release-evidence.json" in text, name


def test_mutating_phases_are_retry_safe_and_effect_states_are_probe_derived():
    candidate = body("release-candidate-publish.yml")
    stable = body("release-stable-publish.yml")
    alias = body("release-image-alias.yml")
    finalize = body("release-finalize.yml")
    assert "release_effects.py classify" in candidate
    assert "release_effects.py plan" in stable
    assert "latest changed outside the authorized compare-and-swap" in stable
    assert "release_effects.py classify" in stable
    assert "release_effects.py plan" in alias
    assert "release_effects.py classify" in alias
    assert "release_effects.py classify" in finalize
    assert "--clobber" not in candidate


def test_candidate_registry_publish_owns_next_atomically():
    reusable = body("_release-registry.yml")
    candidate = body("release-candidate-publish.yml")
    assert "assign-channel" in reusable
    assert "--registry-tag next" in reusable
    assert '"tag": None' not in reusable
    assert "expected_current_version" not in reusable
    assert "/tags/" not in reusable
    assert "expected_next_version" not in candidate
    assert "--clobber" not in candidate
    assert "verify_release_assets" in candidate
    assert 'has("package_descriptor") or has("descriptor_sha256") or has("channel")' in reusable
    assert "registry_publication.py publish-version" in reusable
    assert "build_publish_payload.py" in reusable


def test_bundle_caches_are_scoped_by_descriptor_lock_runtime_and_architecture():
    reusable = body("_release-build.yml")
    assert "release_train.py build-metadata" in reusable
    assert "node-version: ${{ steps.metadata.outputs.runtime_version }}" in reusable
    assert "python-version: ${{ steps.metadata.outputs.runtime_version }}" in reusable
    assert "version: ${{ steps.metadata.outputs.package_manager_version }}" in reusable
    assert "${{ runner.os }}-${{ runner.arch }}-${{ steps.metadata.outputs.lock_sha256 }}" in reusable
    assert "package_json_file" not in reusable
    assert "python-version: '3.12.13'" not in reusable
    assert "lockfile=\"$source_path" not in reusable
    assert "package-manager-cache" not in reusable


def test_release_build_shards_do_not_persist_checkout_or_git_credentials():
    reusable = body("_release-build.yml")
    prepare = yaml.safe_load(body("release-prepare.yml"))
    build = yaml.safe_load(reusable)

    checkout = next(
        step for step in build["jobs"]["build"]["steps"]
        if str(step.get("uses", "")).startswith("actions/checkout@")
    )
    assert checkout["with"]["persist-credentials"] is False
    assert "git config --global" not in reusable
    assert "git config --local" in reusable
    assert "III_CI_APP_ID" not in reusable
    assert "III_CI_APP_PRIVATE_KEY" not in reusable
    assert "WORKERS_REGISTRY_API_KEY" not in reusable

    for job_name in ("prepare", "assemble"):
        checkout = next(
            step for step in prepare["jobs"][job_name]["steps"]
            if str(step.get("uses", "")).startswith("actions/checkout@")
        )
        assert checkout["with"]["persist-credentials"] is False


def test_release_control_app_credentials_are_isolated_to_contract_drift_job():
    workflow = yaml.safe_load(body("ci.yml"))
    drift = workflow["jobs"]["release-contract-drift"]
    assert drift["permissions"] == {"contents": "read"}
    assert drift["runs-on"] == "ubuntu-latest"

    drift_text = str(drift)
    assert "III_CI_APP_ID" in drift_text
    assert "III_CI_APP_PRIVATE_KEY" in drift_text
    assert "release_contract_pin.py" in drift_text
    assert "release-execution.schema.json" in drift_text

    for job_name, job in workflow["jobs"].items():
        if job_name == "release-contract-drift":
            continue
        job_text = str(job)
        assert "III_CI_APP_ID" not in job_text, job_name
        assert "III_CI_APP_PRIVATE_KEY" not in job_text, job_name


def test_release_permissions_are_job_scoped_and_reports_cannot_mutate():
    candidate = yaml.safe_load(body("release-candidate-publish.yml"))
    assert candidate["permissions"] == {"contents": "read"}
    assert candidate["jobs"]["publish"]["permissions"] == {
        "actions": "read",
        "contents": "write",
        "id-token": "write",
        "packages": "write",
    }
    assert candidate["jobs"]["registry"]["permissions"] == {
        "actions": "read",
        "contents": "read",
    }
    assert candidate["jobs"]["report"]["permissions"] == {
        "actions": "read",
        "contents": "read",
        "id-token": "write",
    }

    for name in set(ENTRYPOINTS) - {"release-candidate-publish.yml"}:
        workflow = yaml.safe_load(body(name))
        assert workflow["permissions"] == {}, name
        assert all("permissions" in job for job in workflow["jobs"].values()), name


def test_workflows_parse_as_yaml():
    names = set(ENTRYPOINTS) | REUSABLE | {"release-descriptor-index.yml", "macos-runner-capacity.yml"}
    for name in names:
        assert isinstance(yaml.safe_load(body(name)), dict), name


def test_macos_capacity_gate_proves_three_slots_in_both_release_pools():
    workflow = yaml.safe_load(body("macos-runner-capacity.yml"))
    jobs = workflow["jobs"]
    x64 = [job for job in jobs.values() if job.get("runs-on") == "workers-release-macos-12core"]
    arm = [job for job in jobs.values() if job.get("runs-on") == "workers-release-macos-arm-5core"]
    assert len(x64) == 3
    assert len(arm) == 3
    assert set(jobs["prove-overlap"]["needs"]) == {"macos-slot-1", "macos-slot-2", "macos-slot-3"}
    assert set(jobs["prove-arm-overlap"]["needs"]) == {
        "macos-arm-slot-1", "macos-arm-slot-2", "macos-arm-slot-3",
    }
