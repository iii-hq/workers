import re
from pathlib import Path

import yaml

import _lib
import discover_changed_workers
import validate_worker


ROOT = Path(__file__).resolve().parents[3]
CATALOG = ROOT / "worker-compose.yaml"


def test_root_catalog_has_exact_first_party_and_fixture_counts():
    workers = _lib.read_worker_catalog(CATALOG)
    assert len(workers) == 75
    assert sum(worker.publish for worker in workers.values()) == 69
    assert sum(not worker.publish for worker in workers.values()) == 6


def test_catalog_has_only_new_worker_contract_and_explicit_bundle_files():
    workers = _lib.read_worker_catalog(CATALOG)
    legacy = {"language", "deploy", "manifest", "bin", "scripts", "interface_smoke", "name"}
    for worker_id, worker in workers.items():
        assert set(worker.raw) == {"source", "artifact", "runtime", "registry", "validation"}
        assert not legacy.intersection(worker.raw), worker_id
        for include in worker.artifact.get("include", []):
            assert (worker.path / include).is_file() or include.startswith("dist/"), (
                f"{worker_id}: include must be a source file or deterministic build output: {include}"
            )


def test_public_worker_manifests_remain_valid_and_match_release_catalog():
    errors: list[str] = []
    workers = _lib.read_worker_catalog(CATALOG)
    for worker_id, worker in workers.items():
        assert (worker.path / "iii.worker.yaml").is_file(), worker_id
        if worker.publish:
            validate_worker.validate_public_manifest(worker_id, worker, errors.append)
    assert errors == []


def test_harness_stack_uses_catalog_references_only():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    stack = document["stacks"]["harness"]
    assert set(stack) == {"namespace", "containers"}
    assert len(stack["containers"]) == 13
    for container in stack["containers"].values():
        assert set(container) <= {"worker", "start_after", "config"}
        assert container["worker"].startswith("catalog://")


def test_artifact_kind_drives_ci_language_buckets():
    workers = _lib.read_worker_catalog(CATALOG)
    assert discover_changed_workers.language_of(workers["harness"]) == "rust"
    assert discover_changed_workers.language_of(workers["claude-code"]) == "node"
    assert discover_changed_workers.language_of(workers["scrapling"]) == "python"


def test_rust_frontends_are_explicit_workspace_locked_builds():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    frontends = [
        frontend
        for worker in document["workers"].values()
        for frontend in worker["artifact"].get("frontends", [])
    ]
    assert sum(bool(worker["artifact"].get("frontends")) for worker in document["workers"].values()) == 39
    assert len(frontends) == 42
    for frontend in frontends:
        assert set(frontend) == {
            "workspace_root", "source_path", "runtime", "package_manager", "lockfile",
            "install_command", "build_command", "outputs",
        }
        assert frontend["workspace_root"] == "."
        assert frontend["runtime"] == {"name": "node", "version": "22.20.0"}
        assert frontend["package_manager"] == {"name": "pnpm", "version": "11.13.1"}
        assert frontend["lockfile"] == "pnpm-lock.yaml"
        assert frontend["install_command"] == ["pnpm", "install", "--frozen-lockfile"]
        assert frontend["build_command"] == ["pnpm", "run", "build"]
        assert frontend["outputs"] == ["dist"]


def test_release_toolchains_and_bundle_locks_are_explicit():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    bundles = 0
    for slug, worker in document["workers"].items():
        artifact = worker["artifact"]
        if artifact["kind"] == "rust-binary":
            assert artifact["toolchain"] == {"name": "rust", "version": "1.97.1"}, slug
            continue
        if artifact["kind"] not in {"javascript-bundle", "python-bundle"}:
            continue
        bundles += 1
        assert set(artifact) == {
            "kind", "workspace_root", "runtime", "package_manager", "lockfile",
            "install_command", "build_command", "include",
        }, slug
        lockfile = ROOT / artifact["workspace_root"] / artifact["lockfile"]
        assert lockfile.is_file(), f"{slug}: missing explicit lockfile {lockfile}"
        base_image = worker["runtime"].get("base_image", "")
        assert re.fullmatch(r"[^@\s]+@sha256:[0-9a-f]{64}", base_image), (
            f"{slug}: bundle runtime base image must be immutable"
        )
        if artifact["kind"] == "javascript-bundle":
            assert artifact["runtime"] == {"name": "node", "version": "22.20.0"}
            assert artifact["package_manager"]["name"] in {"pnpm", "npm"}
        else:
            assert artifact["runtime"] == {"name": "python", "version": "3.12.3"}
            assert artifact["package_manager"] == {"name": "uv", "version": "0.12.5"}
    assert bundles == 15


def test_publishable_runtime_prepare_is_offline_and_scrapling_vendors_dependencies():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    network_installers = {"curl", "wget", "pip", "pip3", "npm", "pnpm", "uv", "scrapling"}
    for slug, worker in document["workers"].items():
        if not worker["registry"]["publish"]:
            continue
        for command in worker["runtime"].get("prepare", []):
            assert command[0] not in network_installers, (
                f"{slug}: runtime.prepare must consume only prepared local bytes"
            )

    scrapling = document["workers"]["scrapling"]
    assert "dist/site-packages.tar.gz" in scrapling["artifact"]["include"]
    assert scrapling["artifact"]["install_command"][-1] == "--no-install-project"
    assert scrapling["runtime"]["prepare"] == [[
        "python", "-m", "tarfile", "-e",
        "dist/site-packages.tar.gz", ".release/site-packages",
    ]]
    assert scrapling["runtime"]["environment"]["PYTHONPATH"] == "./.release/site-packages"
