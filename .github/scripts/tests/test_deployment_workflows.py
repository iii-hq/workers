import re
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / ".github" / "workflows"
ENTRYPOINTS = {
    "deploy-prepare.yml": "prepare",
    "deploy-publish.yml": "publish",
    "deploy-verify.yml": "verify",
}
REUSABLE = {"_deploy-build.yml", "_deploy-registry.yml", "_worker-e2e.yml"}


def body(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def test_exact_release_topology_and_no_legacy_wrappers():
    for name in set(ENTRYPOINTS) | REUSABLE:
        assert (WORKFLOWS / name).is_file(), name
    assert {path.name for path in WORKFLOWS.glob("deploy-*.yml")} == (
        set(ENTRYPOINTS) | {"deploy-descriptor-index.yml"}
    )


def test_every_entrypoint_authorizes_and_posts_nominal_release_result():
    for name, phase in ENTRYPOINTS.items():
        text = body(name)
        assert "authorize-dispatch" in text, name
        assert "post-result" in text, name
        assert "--output deployment-result.json" in text, name
        assert f"--phase {phase}" in text, name
        assert "deployment-result-${{ env.DEPLOYMENT_TARGET_ID }}-${{ env.STEP_ID }}-attempt-${{ github.run_attempt }}" in text
        assert f"--workflow '{name}'" in text
        assert "${{ fromJSON(inputs.identity).deployment_batch_id }} · ${{ fromJSON(inputs.identity).deployment_target_id }} · ${{ fromJSON(inputs.identity).step_id }} · ${{ fromJSON(inputs.identity).dispatch_nonce }}" in text
        assert "validate-identity --identity \"$DEPLOYMENT_IDENTITY\"" in text


def test_every_entrypoint_reports_and_uploads_result_even_after_effect_failure():
    for name in ENTRYPOINTS:
        workflow = yaml.safe_load(body(name))
        result_steps = [
            step
            for job in workflow["jobs"].values()
            for step in job.get("steps", [])
            if "deployment-result.json" in str(step)
            and (
                "write-result" in step.get("run", "")
                or "post-result" in step.get("run", "")
                or str(step.get("uses", "")).startswith("actions/upload-artifact@")
            )
        ]
        assert len(result_steps) == 3, name
        assert all(step.get("if") == "always()" for step in result_steps), name


def test_dispatch_inputs_use_one_identity_object_and_fit_github_limit():
    identity_components = {
        "deployment_batch_id", "deployment_target_id", "step_id",
        "attempt_id", "dispatch_nonce", "plan_hash",
    }
    for name in ENTRYPOINTS:
        workflow = yaml.safe_load(body(name))
        inputs = workflow[True]["workflow_dispatch"]["inputs"]
        assert len(inputs) <= 10, name
        assert inputs["identity"] == {"required": True, "type": "string"}
        assert identity_components.isdisjoint(inputs), name
        assert "target_version" in inputs, name
        assert "channel" in inputs, name
        assert "stable_version" not in inputs, name
        assert "prepared_artifact" not in inputs, name


def test_dispatch_values_are_never_rendered_into_shell_source():
    for name in set(ENTRYPOINTS) | {"_deploy-build.yml", "_deploy-registry.yml"}:
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
    for name in set(ENTRYPOINTS) - {"deploy-prepare.yml"}:
        text = body(name)
        assert "worker-compose.yaml" not in text, name
        assert "package_manifest" not in text, name
        assert "iii compose descriptor" not in text, name


def test_deployment_train_never_reads_public_worker_manifests():
    release_files = [WORKFLOWS / name for name in set(ENTRYPOINTS) | REUSABLE]
    release_files.append(WORKFLOWS / "deploy-descriptor-index.yml")
    release_files.extend(
        path for path in (ROOT / ".github" / "scripts").glob("release_*.py")
        if path.name != "deployment_compiler.py"
    )
    for path in release_files:
        assert "iii.worker.yaml" not in path.read_text(encoding="utf-8"), path


def test_descriptor_index_independently_verifies_approved_compiler_bytes():
    text = body("deploy-descriptor-index.yml")
    assert "Verify approved compiler bytes" in text
    assert (
        "APPROVED_COMPILER_DIGEST: "
        "5f720e57d1987b9016a0c2c9b7eaff25a696509506809734d28a88ecc208c364"
    ) in text
    assert 'digest.update(b"iii-workers-deployment-compiler\\0")' in text
    assert 'Path(".github/scripts/deployment_compiler.py").read_bytes()' in text
    assert 'digest.update(b"\\0deployment-descriptor-schema\\0")' in text
    assert 'Path(".github/contracts/deployment-descriptor.schema.json").read_bytes()' in text
    assert "unapproved deployment compiler bytes" in text


def test_release_prepare_fans_one_job_per_compiled_build_unit():
    text = body("deploy-prepare.yml")
    assert "descriptor_run_id" in text
    assert "descriptor_artifact" in text
    assert "deployment_train.py select-descriptor" in text
    assert "iii compose descriptor" not in text
    assert "Build pinned iii compiler" not in text
    assert "frontend-bundles" not in text
    assert "deployment_train.py frontend-metadata" in text
    assert "deployment_train.py build-frontends" in text
    assert "deployment_train.py matrix" in text
    assert "matrix: ${{ fromJSON(needs.prepare.outputs.matrix) }}" in text
    assert "unit: ${{ matrix.unit }}" in text
    assert '--target-version "$TARGET_VERSION"' in text
    assert '--channel "$DEPLOYMENT_CHANNEL"' in text
    assert "package_manifest_version" not in text


def test_release_prepare_reports_failed_source_snapshots_as_absent():
    text = body("deploy-prepare.yml")
    assert 'state=absent; [[ "$outcome" == succeeded ]] && state=present' in text
    assert 'state=unknown; [[ "$outcome" == succeeded ]] && state=present' not in text


def test_release_build_validates_rust_targets_and_oci_platforms():
    text = body("_deploy-build.yml")
    assert '(.target // .platform // "none") == $target' in text


def test_prepare_uploads_inventory_without_booting_the_worker():
    text = body("deploy-prepare.yml")
    workflow = yaml.safe_load(text)
    assert "adapter" not in workflow["jobs"]
    assert "deployment-prepared-${{ env.DEPLOYMENT_TARGET_ID }}" in text
    assert "deployment_interface.py" not in text
    assert "collect_worker_interface.py" not in text
    assert "install.iii.dev" not in text


def test_all_post_prepare_phases_verify_prepared_bytes_and_report_them():
    for name in set(ENTRYPOINTS) - {"deploy-prepare.yml"}:
        text = body(name)
        assert "deployment_train.py verify-prepared" in text, name
        assert "prepared-artifacts.json" in text, name
        assert "deployment-evidence.json" not in text, name
    registry = body("_deploy-registry.yml")
    assert "deployment_train.py verify-prepared" in registry
    assert "deployment_interface.py" not in registry


def test_publish_is_retry_safe_and_effect_states_are_probe_derived():
    publish = body("deploy-publish.yml")
    workflow = yaml.safe_load(publish)
    checkout = workflow["jobs"]["publish"]["steps"][0]
    assert "ref" not in checkout["with"]
    assert checkout["with"]["fetch-depth"] == 0
    assert "deployment_effects.py classify" in publish
    assert 'value=sys.argv[1].strip()' in publish
    assert 'all(.[]; .state == "absent" or .state == "present" or .state == "unknown")' in publish
    registry = body("_deploy-registry.yml")
    assert "registry_publication.py assign-channel" in registry
    assert "registry_publication.py advance-next-floor" in registry
    assert "expected-current-version" in registry
    assert "expected-next-version" in registry
    assert "--clobber" not in publish


def test_publish_separates_immutable_version_from_explicit_channel_cas():
    reusable = body("_deploy-registry.yml")
    publish = body("deploy-publish.yml")
    assert "assign-channel" in reusable
    assert "target_version" in reusable
    assert "channel" in reusable
    assert '"tag": None' not in reusable
    assert "expected_current_version" in reusable
    assert "expected_next_version" in reusable
    assert "--clobber" not in publish
    assert "verify_release_assets" in publish
    assert 'has("package_descriptor") or has("descriptor_sha256") or has("channel")' in reusable
    assert "registry_publication.py publish-version" in reusable
    assert "build_publish_payload.py" in reusable
    assert '--registry-tag "$DEPLOYMENT_CHANNEL"' in reusable
    assert "Move requested channel with compare-and-swap" in reusable
    assert "Move the requested OCI channel to the immutable digest" in publish
    assert '"$DEPLOYMENT_CHANNEL" == next || "$TARGET_VERSION" == *-*' in publish
    assert '"$RELEASE_CHANNEL" == next || "$TARGET_VERSION" == *-*' in body("deploy-verify.yml")


def test_bundle_caches_are_scoped_by_descriptor_lock_runtime_and_architecture():
    reusable = body("_deploy-build.yml")
    assert "deployment_train.py build-metadata" in reusable
    assert "node-version: ${{ steps.metadata.outputs.runtime_version }}" in reusable
    assert "python-version: ${{ steps.metadata.outputs.runtime_version }}" in reusable
    assert "version: ${{ steps.metadata.outputs.package_manager_version }}" in reusable
    assert "${{ runner.os }}-${{ runner.arch }}-${{ steps.metadata.outputs.lock_sha256 }}" in reusable
    assert "package_json_file: ${{ steps.metadata.outputs.source_path }}/package.json" in reusable
    assert "python-version: '3.12.13'" not in reusable
    assert "lockfile=\"$source_path" not in reusable
    assert "package-manager-cache" not in reusable


def test_release_build_shards_do_not_persist_checkout_or_git_credentials():
    reusable = body("_deploy-build.yml")
    prepare = yaml.safe_load(body("deploy-prepare.yml"))
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
    assert "deployment_contract_pin.py" in drift_text
    assert "deployment-execution.schema.json" in drift_text

    for job_name, job in workflow["jobs"].items():
        if job_name == "release-contract-drift":
            continue
        job_text = str(job)
        assert "III_CI_APP_ID" not in job_text, job_name
        assert "III_CI_APP_PRIVATE_KEY" not in job_text, job_name


def test_release_permissions_are_job_scoped_and_reports_cannot_mutate():
    publish = yaml.safe_load(body("deploy-publish.yml"))
    assert publish["permissions"] == {"contents": "read"}
    assert publish["jobs"]["publish"]["permissions"] == {
        "actions": "read",
        "contents": "write",
        "id-token": "write",
        "packages": "write",
    }
    assert publish["jobs"]["registry"]["permissions"] == {
        "actions": "read",
        "contents": "read",
    }
    assert publish["jobs"]["report"]["permissions"] == {
        "actions": "read",
        "contents": "read",
        "id-token": "write",
        "packages": "read",
    }

    for name in set(ENTRYPOINTS) - {"deploy-publish.yml"}:
        workflow = yaml.safe_load(body(name))
        assert workflow["permissions"] == {}, name
        assert all("permissions" in job for job in workflow["jobs"].values()), name


def test_workflows_parse_as_yaml():
    names = set(ENTRYPOINTS) | REUSABLE | {"deploy-descriptor-index.yml", "macos-runner-capacity.yml"}
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
