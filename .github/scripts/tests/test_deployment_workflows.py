import re
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / ".github" / "workflows"
# The convergent pipeline: one entrypoint that builds and uploads immutable
# bytes, one reusable build shard, and the descriptor checkpoint. Release
# Control publishes versions, channels, releases and image tags from the
# attested build manifest, so there is no publish/finalize/verify workflow.
BUILD = "build.yml"
REUSABLE = {"_deploy-build.yml", "_worker-e2e.yml"}
DEPLOY_TRAIN = {BUILD, "_deploy-build.yml", "deploy-descriptor-index.yml"}


def body(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def test_the_train_is_exactly_build_plus_the_descriptor_checkpoint():
    for name in DEPLOY_TRAIN | REUSABLE:
        assert (WORKFLOWS / name).is_file(), name
    assert {path.name for path in WORKFLOWS.glob("deploy-*.yml")} == {"deploy-descriptor-index.yml"}
    assert not (WORKFLOWS / "_deploy-registry.yml").exists()
    assert not (WORKFLOWS / "_deploy-registry-finalize.yml").exists()


def test_no_workflow_talks_to_release_control_or_the_registry():
    for name in DEPLOY_TRAIN | REUSABLE:
        text = body(name)
        assert "deployment_control_contract.py" not in text, name
        assert "RELEASE_CONTROL_API_URL" not in text, name
        assert "registry_publication.py" not in text, name
        assert "WORKERS_REGISTRY_API_KEY" not in text, name
        assert "deployment-result" not in text, name


def test_dispatch_values_are_never_rendered_into_shell_source():
    for name in DEPLOY_TRAIN:
        workflow = yaml.safe_load(body(name))
        for job in workflow["jobs"].values():
            for step in job.get("steps", []):
                command = step.get("run", "")
                assert "${{ inputs." not in command, f"{name}: {step.get('name', 'unnamed')}"


def test_new_release_workflows_pin_third_party_actions_by_sha():
    for name in DEPLOY_TRAIN | REUSABLE:
        for line in body(name).splitlines():
            if "uses:" not in line or "./.github/" in line:
                continue
            reference = line.split("uses:", 1)[1].strip().split()[0]
            assert re.search(r"@[0-9a-f]{40}$", reference), f"{name}: mutable action {reference}"


def test_deployment_train_never_reads_public_worker_manifests():
    release_files = [WORKFLOWS / name for name in DEPLOY_TRAIN | REUSABLE]
    release_files.extend(
        path for path in (ROOT / ".github" / "scripts").glob("release_*.py")
        if path.name != "deployment_compiler.py"
    )
    for path in release_files:
        assert "iii.worker.yaml" not in path.read_text(encoding="utf-8"), path


def test_build_fans_one_job_per_compiled_build_unit():
    text = body(BUILD)
    assert "descriptor_run_id" in text
    assert "deployment_train.py select-descriptor" in text
    assert "iii compose descriptor" not in text
    assert "Build pinned iii compiler" not in text
    assert "deployment_train.py frontend-metadata" in text
    assert "deployment_train.py build-frontends" in text
    assert "deployment_train.py matrix" in text
    # One profile, one matrix: the descriptor's build_units are the whole fan-out.
    assert "--profile" not in text
    assert "matrix: ${{ fromJSON(needs.resolve.outputs.matrix) }}" in text
    assert "unit: ${{ matrix.unit }}" in text
    # Versions and channels are Release Control's, never the build's.
    assert "target_version" not in text
    assert "DEPLOYMENT_CHANNEL" not in text


def test_build_captures_interface_from_prepared_bytes():
    text = body(BUILD)
    workflow = yaml.safe_load(text)
    capture_jobs = [job for job in workflow["jobs"].values() if "deployment_interface.py stage" in str(job)]
    assert len(capture_jobs) == 1
    assert set(capture_jobs[0]["needs"]) == {"resolve", "build"}
    assert "deployment_interface.py verify-evidence" in text
    assert "deployment-interface.json" in text
    interface_text = str(capture_jobs[0])
    assert "worker-compose.yaml" not in interface_text


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


def test_macos_capacity_gate_proves_three_slots_in_the_release_pool():
    """Only Apple Silicon is left to prove: the Intel pool was retired with
    `x86_64-apple-darwin`, so a slot count for it would assert against a pool
    that no longer exists."""
    workflow = yaml.safe_load(body("macos-runner-capacity.yml"))
    jobs = workflow["jobs"]
    pools = {job.get("runs-on") for job in jobs.values()}
    assert "workers-release-macos-12core" not in pools
    arm = [job for job in jobs.values() if job.get("runs-on") == "workers-release-macos-arm-5core"]
    assert len(arm) == 3
    assert set(jobs["prove-arm-overlap"]["needs"]) == {
        "macos-arm-slot-1", "macos-arm-slot-2", "macos-arm-slot-3",
    }


def test_build_shards_do_not_persist_checkout_or_git_credentials():
    reusable = body("_deploy-build.yml")
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

    convergent = yaml.safe_load(body(BUILD))
    for job in convergent["jobs"].values():
        for step in job.get("steps", []):
            if str(step.get("uses", "")).startswith("actions/checkout@"):
                assert step["with"]["persist-credentials"] is False


def test_workflows_parse_as_yaml():
    names = DEPLOY_TRAIN | REUSABLE | {"macos-runner-capacity.yml"}
    for name in names:
        assert isinstance(yaml.safe_load(body(name)), dict), name


def test_deploy_train_workflows_never_install_python_packages() -> None:
    # The self-hosted release runners are not guaranteed to have pip on PATH:
    # a stray `pip install` under `set -euo pipefail` killed ten workers in the
    # first nightly batch. The deploy train is jq/python-stdlib only; pyyaml is
    # legitimately installed solely by deploy-descriptor-index.yml.
    for name in sorted((DEPLOY_TRAIN | REUSABLE) - {"_worker-e2e.yml", "deploy-descriptor-index.yml"}):
        assert "pip install" not in body(name), name


def test_build_workflow_only_builds_and_uploads_immutable_bytes():
    text = body(BUILD)
    workflow = yaml.safe_load(text)
    assert set(workflow[True]) == {"workflow_dispatch", "workflow_call"}
    for trigger in ("workflow_dispatch", "workflow_call"):
        inputs = workflow[True][trigger]["inputs"]
        assert set(inputs) == {"worker", "source_sha", "descriptor_run_id", "correlation_id"}, trigger
        assert inputs["correlation_id"]["required"] is False
    # No Release Control identity, authorization or result reporting.
    assert "deployment_control_contract.py" not in text
    assert "RELEASE_CONTROL_API_URL" not in text
    assert "fromJSON(inputs.identity)" not in text
    assert "target_version" not in text
    # No version or channel publication: Release Control owns those.
    assert "registry_publication.py" not in text
    assert "assign-channel" not in text
    assert "_deploy-registry" not in text
    assert "--clobber" not in text
    assert "--latest" not in text
    assert workflow["concurrency"] == {
        "group": "build-${{ inputs.worker }}-${{ inputs.source_sha }}",
        "cancel-in-progress": False,
    }
    assert list(workflow["jobs"]) == ["resolve", "build", "assemble", "upload", "manifest"]
    assert workflow["jobs"]["build"]["uses"] == "./.github/workflows/_deploy-build.yml"
    assert workflow["jobs"]["build"]["with"]["prepared_sha"] == "${{ inputs.source_sha }}"
    assert workflow["jobs"]["build"]["strategy"]["matrix"] == "${{ fromJSON(needs.resolve.outputs.matrix) }}"
    assert set(workflow["jobs"]["assemble"]["needs"]) == {"resolve", "build"}
    assert set(workflow["jobs"]["upload"]["needs"]) == {"resolve", "assemble"}
    assert set(workflow["jobs"]["manifest"]["needs"]) == {"resolve", "assemble", "upload"}
    # Content-addressed surfaces only.
    assert "BUILD_TAG: build-${{ inputs.source_sha }}" in text
    assert 'gh release create "$BUILD_TAG" --target "$SOURCE_SHA" --prerelease' in text
    assert 'docker buildx imagetools create --tag "$base:$BUILD_TAG"' in text
    assert 'sudo skopeo login --authfile "$authfile"' in text
    assert "--password-stdin" in text
    assert '--password "$GH_TOKEN"' not in text
    assert 'sudo skopeo inspect --authfile "$authfile"' in text
    assert 'sudo skopeo copy --authfile "$authfile"' in text
    assert "immutable assets are never overwritten" in text
    assert "immutable images are never overwritten" in text
    assert "deployment_train.py select-descriptor" in text
    assert "run-tooling/.github/scripts/deployment_interface.py stage" in text
    assert '"$tooling/deployment_interface.py" snapshot' in text
    assert '"$tooling/deployment_interface.py" build-evidence' in text
    assert "deployment_interface.py verify-evidence" in text
    assert "build_manifest.py plan" in text
    assert "build_manifest.py write" in text


def test_build_workflow_checks_out_the_exact_source_sha_without_credentials():
    workflow = yaml.safe_load(body(BUILD))
    for job_name, job in workflow["jobs"].items():
        for step in job.get("steps", []):
            if str(step.get("uses", "")).startswith("actions/checkout@"):
                assert step["with"]["persist-credentials"] is False, job_name
                assert "ref" in step["with"], job_name
    resolve_checkout = workflow["jobs"]["resolve"]["steps"][0]
    assert resolve_checkout["with"]["ref"] == "${{ inputs.source_sha }}"
    assert 'test "$(git rev-parse HEAD)" = "$SOURCE_SHA"' in body(BUILD)
    assert "[[ \"$SOURCE_SHA\" =~ ^[0-9a-f]{40}$ ]]" in body(BUILD)


def test_build_workflow_permissions_are_minimal_per_job():
    workflow = yaml.safe_load(body(BUILD))
    assert workflow["permissions"] == {}
    jobs = workflow["jobs"]
    assert jobs["resolve"]["permissions"] == {"actions": "read", "contents": "read"}
    assert jobs["build"]["permissions"] == {"actions": "read", "contents": "read", "id-token": "write"}
    assert jobs["assemble"]["permissions"] == {"actions": "read", "contents": "read"}
    assert jobs["upload"]["permissions"] == {"actions": "read", "contents": "write", "packages": "write"}
    assert jobs["manifest"]["permissions"] == {
        "actions": "read", "attestations": "write", "contents": "read", "id-token": "write",
    }
    for name, job in jobs.items():
        if name != "upload":
            assert job["permissions"].get("contents") != "write", name
            assert "packages" not in job["permissions"], name
        if name != "manifest":
            assert "attestations" not in job["permissions"], name


def test_build_manifest_is_attested_and_only_written_after_every_upload_succeeds():
    workflow = yaml.safe_load(body(BUILD))
    manifest = workflow["jobs"]["manifest"]
    assert "if" not in manifest
    assert "always()" not in str(manifest)
    steps = manifest["steps"]
    attest = next(step for step in steps if str(step.get("uses", "")).startswith("actions/attest-build-provenance@"))
    assert attest["with"] == {"subject-path": "manifest.json"}
    upload = next(step for step in steps if str(step.get("uses", "")).startswith("actions/upload-artifact@"))
    assert upload["with"]["name"] == "build-manifest"
    assert upload["with"]["path"] == "manifest.json"
    assert upload["with"]["if-no-files-found"] == "error"
    assert upload["with"]["retention-days"] == 90
    assert steps.index(attest) < steps.index(upload)
    for name in ("resolve", "assemble", "upload"):
        assert "always()" not in str(workflow["jobs"][name]), name
