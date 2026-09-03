import re
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / ".github" / "workflows"
ENTRYPOINTS = {
    "deploy-prepare.yml": "prepare",
    "deploy-publish.yml": "publish",
    "deploy-verify.yml": "verify",
    "deploy-finalize.yml": "finalize",
}
REUSABLE = {
    "_deploy-build.yml",
    "_deploy-registry.yml",
    "_deploy-registry-finalize.yml",
    "_worker-e2e.yml",
}


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
        if name == "deploy-finalize.yml":
            # Finalization is latest-only by construction: the channel is a
            # fixed workflow constant, never a dispatch degree of freedom.
            assert "channel" not in inputs, name
        else:
            assert "channel" in inputs, name
        assert "stable_version" not in inputs, name
        assert "prepared_artifact" not in inputs, name


def test_finalize_dispatch_carries_the_full_rc_promotion_context():
    workflow = yaml.safe_load(body("deploy-finalize.yml"))
    assert set(workflow[True]["workflow_dispatch"]["inputs"]) == {
        "identity", "worker", "source_sha", "source_rc_version",
        "target_version", "expected_current_version", "expected_next_version",
        "descriptor_sha256", "prepared_run_id", "source_target_id",
    }
    assert "DEPLOYMENT_CHANNEL: latest" in body("deploy-finalize.yml")


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


def test_descriptor_index_is_automatic_and_bound_to_the_exact_main_commit():
    text = body("deploy-descriptor-index.yml")
    workflow = yaml.safe_load(text)
    assert workflow[True] == {"push": {"branches": ["main"]}}
    assert "workflow_dispatch" not in text
    assert "APPROVED_COMPILER_DIGEST" not in text
    assert "Verify approved compiler bytes" not in text
    assert "ref: ${{ github.sha }}" in text
    assert "--source-sha '${{ github.sha }}'" in text
    assert "--compiler-commit" not in text
    assert "name: deployment-descriptor-index-${{ github.sha }}" in text


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
    # One profile, one matrix: the descriptor's build_units are the whole fan-out.
    assert "--profile" not in text
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


def test_release_build_falls_back_when_optional_sccache_is_unavailable():
    workflow = yaml.safe_load(body("_deploy-build.yml"))
    steps = workflow["jobs"]["build"]["steps"]
    by_name = {step.get("name"): step for step in steps}

    assert by_name["Authorize cache access with GitHub OIDC"]["continue-on-error"] is True
    assert by_name["Install sccache"]["continue-on-error"] is True
    assert "sccache rustc -vV" in by_name["Probe optional sccache backend"]["run"]
    assert "wrapper=" in by_name["Probe optional sccache backend"]["run"]
    assert by_name["Build immutable artifact"]["env"]["RUSTC_WRAPPER"] == (
        "${{ steps.sccache.outputs.wrapper }}"
    )
    assert "RUSTC_WRAPPER" not in workflow["env"]


def test_prepare_captures_interface_from_prepared_bytes():
    text = body("deploy-prepare.yml")
    workflow = yaml.safe_load(text)
    capture_jobs = [
        job
        for job in workflow["jobs"].values()
        if "deployment_interface.py stage" in str(job)
    ]
    assert len(capture_jobs) == 1
    capture_job = capture_jobs[0]
    assert set(capture_job["needs"]) == {"prepare", "assemble"}
    assert "deployment_interface.py stage" in text
    assert "deployment_interface.py snapshot" in text
    assert "deployment_interface.py build-evidence" in text
    assert "deployment-prepared-${{ env.DEPLOYMENT_TARGET_ID }}" in text
    interface_text = str(capture_job)
    assert "inputs.source_sha" not in interface_text
    assert "worker-compose.yaml" not in interface_text


def test_prepare_upload_requires_interface_evidence():
    workflow = yaml.safe_load(body("deploy-prepare.yml"))
    uploads = [
        step
        for job in workflow["jobs"].values()
        for step in job.get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
        and str(step.get("with", {}).get("name", "")).startswith("deployment-prepared-")
    ]
    assert len(uploads) == 1
    upload = uploads[0]
    assert upload["with"]["if-no-files-found"] == "error"
    uploaded_path = str(upload["with"]["path"])
    assert "deployment-interface.json" in uploaded_path or uploaded_path.rstrip("/").endswith("deploy-prepared")


def test_registry_publishers_consume_interface_evidence():
    for name in ("_deploy-registry.yml", "_deploy-registry-finalize.yml"):
        text = body(name)
        assert "deployment_interface.py verify-evidence" in text, name
        assert "deployment-evidence.json" in text, name
        assert "deployment-interface.json" in text, name


def test_registry_publishers_never_synthesize_an_empty_interface():
    for name in ("_deploy-registry.yml", "_deploy-registry-finalize.yml"):
        text = body(name)
        assert '"functions": []' not in text, name
        assert '"triggers": []' not in text, name
        assert "deployment-interface.json" in text, name
        assert "--interface-json interface.json" in text, name


def test_publish_is_retry_safe_and_effect_states_are_probe_derived():
    publish = body("deploy-publish.yml")
    workflow = yaml.safe_load(publish)
    checkout = workflow["jobs"]["publish"]["steps"][0]
    assert "ref" not in checkout["with"]
    assert checkout["with"]["fetch-depth"] == 0
    # Effect states pass straight from the probes to the result: with no
    # pre-mutation probe there is no classification step to reinterpret them.
    assert "deployment_effects.py" not in publish
    assert 'echo "github_state=$github_state"' in publish
    assert 'echo "image_channel_state=$image_channel_state"' in publish
    assert "PUBLISH_GITHUB_STATE" in publish
    assert "\n          GITHUB_STATE:" not in publish
    assert 'value=sys.argv[1].strip()' in publish
    assert 'all(.[]; .state == "absent" or .state == "present" or .state == "unknown")' in publish
    registry = body("_deploy-registry.yml")
    assert registry.count("registry_publication.py assign-channel") == 2
    assert "expected-current-version" in registry
    assert "EXPECTED_MIRROR_VERSION" in registry
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
    assert "jq 'has(\"tag\")' payload.json" in reusable
    assert "has(\\\"tag\\\")" not in reusable
    assert "MIRROR_CHANNEL: ${{ inputs.channel == 'latest' && 'next' || 'latest' }}" in reusable
    assert '--registry-tag "$DEPLOYMENT_CHANNEL"' in reusable
    assert '--registry-tag "$MIRROR_CHANNEL"' in reusable
    assert "Move requested channel with compare-and-swap" in reusable
    assert "Mirror the other channel with compare-and-swap" in reusable
    assert "Move the requested OCI channel to the immutable digest" in publish
    assert '"$DEPLOYMENT_CHANNEL" == next || "$TARGET_VERSION" == *-*' in publish
    assert '"$RELEASE_CHANNEL" == next || "$TARGET_VERSION" == *-*' in body("deploy-verify.yml")


def test_finalize_serializes_with_publish_per_worker():
    # Serialization is load-bearing: a finalize racing a publish for the same
    # worker would interleave the channel compare-and-swap moves.
    finalize = yaml.safe_load(body("deploy-finalize.yml"))
    publish = yaml.safe_load(body("deploy-publish.yml"))
    assert finalize["concurrency"]["group"] == "deployment-${{ inputs.worker }}"
    assert finalize["concurrency"] == publish["concurrency"]


def test_prepared_artifacts_outlive_the_soak_window():
    # Verified rc bytes are the finalization fast path: prepare uploads them and
    # finalize re-uploads the very same bytes, so both must keep them 90 days.
    for name in ("deploy-prepare.yml", "deploy-finalize.yml"):
        workflow = yaml.safe_load(body(name))
        uploads = [
            step
            for job in workflow["jobs"].values()
            for step in job.get("steps", [])
            if str(step.get("uses", "")).startswith("actions/upload-artifact@")
            and str(step.get("with", {}).get("name", "")).startswith("deployment-prepared-")
        ]
        assert len(uploads) == 1, name
        assert uploads[0]["with"]["retention-days"] == 90, name


def test_finalize_promotes_the_rc_bytes_verbatim_with_no_supplemental_build():
    # Every deployment builds every target, Windows included, so what soaked on
    # @next is already the whole release: finalization promotes those bytes
    # verbatim. A supplemental build here would publish @latest bytes that no
    # rc ever exercised.
    text = body("deploy-finalize.yml")
    workflow = yaml.safe_load(text)
    assert "deployment_train.py finalize" in text
    assert "build_supplemental" not in workflow["jobs"]
    assert "build_supplemental" not in text
    assert "_deploy-build.yml" not in text
    assert "deployment_train.py matrix" not in text
    assert "assemble-stable" not in text
    assert "stable-delta" not in text
    assert "--profile" not in text


def test_finalize_fallback_rebuilds_evidence_from_the_exact_candidate_interface():
    workflow = yaml.safe_load(body("deploy-finalize.yml"))
    fallback_steps = [
        step
        for job in workflow["jobs"].values()
        for step in job.get("steps", [])
        if step.get("if") == "steps.prepared.outcome != 'success'"
        and "deployment_interface.py snapshot" in step.get("run", "")
    ]
    assert len(fallback_steps) == 1
    command = fallback_steps[0]["run"]
    assert "api.workers.iii.dev/w/$WORKER?version=$SOURCE_RC_VERSION" in command
    assert "deployment_interface.py build-evidence" in command
    assert "deployment_interface.py verify-evidence" in command
    assert "deployment-interface.json" in command
    assert "TARGET_VERSION" not in command


def test_registry_finalize_is_atomic_and_never_moves_channels_piecewise():
    text = body("_deploy-registry-finalize.yml")
    assert "registry_publication.py finalize-release" in text
    assert "registry_publication.py publish-version" in text
    assert "deployment_train.py verify-prepared" in text
    assert "advance-next-floor" not in text
    assert "assign-channel" not in text


def test_repo_latest_is_granted_only_after_the_registry_receipt():
    workflow = yaml.safe_load(body("deploy-finalize.yml"))
    assert "registry" in workflow["jobs"]["promote"]["needs"]
    assert "--latest=false" in body("deploy-finalize.yml")


def test_release_build_restores_frontends_with_a_portable_copy():
    text = body("_deploy-build.yml")
    assert "shutil.copytree('.release-frontend', '.', dirs_exist_ok=True)" in text
    # rsync is not guaranteed on the Windows and macOS release runners: it may
    # be named only in comments, never invoked.
    for line in text.splitlines():
        if "rsync" in line:
            assert line.lstrip().startswith("#"), line


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

def test_deploy_train_workflows_never_install_python_packages() -> None:
    # The self-hosted release runners are not guaranteed to have pip on PATH:
    # a stray `pip install` under `set -euo pipefail` killed ten workers in the
    # first nightly batch. The deploy train is jq/python-stdlib only; pyyaml is
    # legitimately installed solely by deploy-descriptor-index.yml (and the
    # e2e harness workflow, which is not part of the train).
    train = (set(ENTRYPOINTS) | REUSABLE) - {"_worker-e2e.yml"}
    train |= {path.name for path in WORKFLOWS.glob("deploy-*.yml")} - {"deploy-descriptor-index.yml"}
    for name in sorted(train):
        assert "pip install" not in body(name), name
