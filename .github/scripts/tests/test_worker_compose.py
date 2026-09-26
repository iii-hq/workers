import tomllib
from pathlib import Path

import yaml

import _lib
import deployment_targets
import discover_changed_workers
import validate_worker


ROOT = Path(__file__).resolve().parents[3]
CATALOG = ROOT / ".deploy" / "workers.yaml"


def test_catalog_has_only_release_build_contract_and_explicit_bundle_files():
    workers = _lib.read_worker_catalog(CATALOG)
    legacy = {"language", "deploy", "manifest", "bin", "scripts", "interface_smoke", "registry_interface", "name"}
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
    assert discover_changed_workers.language_of(workers["hermes"]) == "python"


def test_rust_frontends_are_explicit_workspace_locked_builds():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    frontends = [
        frontend
        for worker in document["workers"].values()
        for frontend in worker["artifact"].get("frontends", [])
    ]
    assert sum(bool(worker["artifact"].get("frontends")) for worker in document["workers"].values()) == 50
    assert len(frontends) == 54
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


def test_rust_release_declares_every_frontend_its_build_compiles():
    # PR CI finds ui/ and web/ bundles on disk and installs pnpm for them; the
    # release build installs pnpm only for declared frontends. An undeclared
    # bundle passes every PR and then panics in build.rs at release time.
    def build_crates(crate: Path, seen: set[Path]) -> set[Path]:
        if crate not in seen:
            seen.add(crate)
            manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
            for table in [manifest, *manifest.get("target", {}).values()]:
                for section in ("dependencies", "build-dependencies"):
                    for dependency in table.get(section, {}).values():
                        if isinstance(dependency, dict) and "path" in dependency:
                            build_crates((crate / dependency["path"]).resolve(), seen)
        return seen

    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    for slug, worker in document["workers"].items():
        artifact = worker["artifact"]
        if artifact["kind"] != "rust-binary":
            continue
        compiled = {
            (crate / bundle).relative_to(ROOT).as_posix()
            for crate in build_crates((ROOT / worker["source"]["path"]).resolve(), set())
            for bundle in ("ui", "web")
            if (crate / bundle / "package.json").is_file()
        }
        declared = {frontend["source_path"] for frontend in artifact.get("frontends", [])}
        assert compiled <= declared, f"{slug}: declare {sorted(compiled - declared)} in artifact.frontends"


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
    assert bundles == 14


def test_claude_code_release_builds_its_ui_from_the_root_workspace():
    # A private claude-code workspace re-lists ../packages/console-ui, whose
    # `catalog:` specifiers only the root workspace defines, and the frozen
    # install then rejects the lockfile. The ui lives in the root workspace.
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    artifact = document["workers"]["claude-code"]["artifact"]
    root_workspace = yaml.safe_load((ROOT / "pnpm-workspace.yaml").read_text(encoding="utf-8"))

    assert artifact["workspace_root"] == "claude-code"
    assert artifact["install_command"] == ["pnpm", "install", "--ignore-workspace", "--frozen-lockfile"]
    assert not (ROOT / "claude-code" / "pnpm-workspace.yaml").exists()
    assert "claude-code/ui" in root_workspace["packages"]


def test_worker_bundle_start_commands_target_packaged_entrypoints():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))

    for worker_id in (
        "claude-code",
        "cursor",
        "opencode",
        "opengantry",
        "openwiki",
        "pi",
        "vscode",
    ):
        worker = document["workers"][worker_id]
        manifest = yaml.safe_load(
            (ROOT / worker["source"]["path"] / "iii.worker.yaml").read_text(
                encoding="utf-8"
            )
        )
        start = manifest["scripts"]["start"]

        assert start.startswith("node ./"), worker_id
        assert start.removeprefix("node ./") in worker["artifact"]["include"], worker_id


def test_hermes_release_image_supports_both_linux_architectures():
    document = yaml.safe_load(CATALOG.read_text(encoding="utf-8"))
    artifact = document["workers"]["hermes"]["artifact"]

    assert artifact == {
        "kind": "oci-image",
        "context": ".",
        "dockerfile": "Dockerfile",
        "platforms": ["linux/amd64", "linux/arm64"],
    }


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
    # before the cutover. New Unix-only workers must declare a reviewed exception.
    assert without_windows == {
        "acp",
        "compose-ui",
        "code-runner",
        "context-manager",
        "lsp",
        "quick-tunnel",  # New Unix-only worker; child lifecycle is not supported on Windows.
        "sandbox-code-runner",
        "ide",
        # New workers, never published for Windows: llama.cpp from source.
        "judge-semif",
        "judge-decider",
        "judge-laya",
        "voice",
        "workflow",
    }
