from pathlib import Path

import yaml

import _lib
import deployment_targets
import discover_changed_workers
import validate_worker


ROOT = Path(__file__).resolve().parents[3]
CATALOG = ROOT / ".deploy" / "workers.yaml"


def test_private_catalog_has_exact_first_party_and_fixture_counts():
    workers = _lib.read_worker_catalog(CATALOG)
    assert len(workers) == 76
    assert sum(worker.publish for worker in workers.values()) == 70
    assert sum(not worker.publish for worker in workers.values()) == 6


def test_catalog_has_only_release_build_contract_and_explicit_bundle_files():
    workers = _lib.read_worker_catalog(CATALOG)
    legacy = {"language", "deploy", "manifest", "bin", "scripts", "interface_smoke", "name"}
    for worker_id, worker in workers.items():
        assert set(worker.raw) == {"source", "artifact", "publish"}
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


def test_public_harness_compose_uses_the_current_cli_contract():
    compose = yaml.safe_load((ROOT / "harness" / "worker-compose.yaml").read_text(encoding="utf-8"))
    assert "workers" not in compose
    assert "stacks" not in compose
    assert set(compose) >= {"namespace", "containers"}


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
        if artifact["kind"] == "javascript-bundle":
            assert artifact["runtime"] == {"name": "node", "version": "22.20.0"}
            assert artifact["package_manager"]["name"] in {"pnpm", "npm"}
        else:
            assert artifact["runtime"] == {"name": "python", "version": "3.12.3"}
            assert artifact["package_manager"] == {"name": "uv", "version": "0.12.5"}
    assert bundles == 16


def test_scrapling_release_bundle_vendors_dependencies():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    scrapling = document["workers"]["scrapling"]
    assert "dist/site-packages.tar.gz" in scrapling["artifact"]["include"]
    assert scrapling["artifact"]["install_command"][-1] == "--no-install-project"


def test_every_rust_worker_ships_windows_or_justifies_its_absence():
    """Windows must never be lost by silent omission again.

    The deployment cutover dropped all three msvc triples from the catalog
    without anyone declaring it, and the loss only surfaced when `latest`
    stayed pinned to a pre-cutover version. A worker now either builds the
    full Windows matrix or says in writing why it cannot.
    """
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    windows = set(deployment_targets.WINDOWS_TARGETS)
    without_windows = set()
    for slug, entry in document["workers"].items():
        artifact = entry["artifact"]
        if artifact["kind"] != "rust-binary":
            continue
        declared = windows.intersection(artifact["targets"])
        exception = artifact.get("windows_exception")
        if declared:
            assert declared == windows, f"{slug}: partial Windows matrix {sorted(declared)}"
            assert not exception, f"{slug}: declares Windows targets and an exception"
        else:
            assert isinstance(exception, str) and exception.strip(), f"{slug}: silent Windows omission"
            without_windows.add(slug)
    # Audited against the Registry: every other worker published msvc binaries
    # before the cutover, so anything joining this set is a regression.
    assert without_windows == {
        "acp",
        "code-runner",
        "context-manager",
        "editor",
        "lsp",
        "sandbox-code-runner",
        "shell",
        "workflow",
    }
