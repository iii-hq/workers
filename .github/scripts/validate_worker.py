#!/usr/bin/env python3
"""Validate one publishable private release entry and its public manifest."""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile
import tomllib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402
import worker_screenshots  # noqa: E402


BINARY_NAME_EXCEPTIONS = frozenset({"acp"})

BUNDLE_PRESET_IMAGES = frozenset({
    "docker.io/iiidev/python:latest",
    "docker.io/iiidev/node:latest",
})


def release_profile_error(worker: str, manifest_path: pathlib.Path) -> str | None:
    """Return why a rust-binary manifest lacks an explicit `[profile.release]`, or None.

    Every worker is its own Cargo workspace and ships the binary its root
    manifest builds, so Cargo's default release profile (no cross-crate LTO,
    16 codegen units, symbol table kept) would otherwise reach users.
    """
    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        return f"{worker}: could not parse {manifest_path.name}: {error}"
    if not isinstance(manifest.get("profile", {}).get("release"), dict):
        return (
            f"{worker}/{manifest_path.name} must declare [profile.release] "
            '(the shared one is lto = "thin", codegen-units = 1, strip = true)'
        )
    return None


RUST_EDITION = "2024"

# Crates first-party manifests must not depend on: the deprecated serde_yaml
# moved to serde_yaml_ng, and serde_yml is unmaintained (RUSTSEC-2025-0068).
DENIED_CRATES = {
    "serde_yaml": "serde_yaml is deprecated upstream; use serde_yaml_ng",
    "serde_yml": "serde_yml is unmaintained (RUSTSEC-2025-0068); use serde_yaml_ng",
}

DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def _dependency_names(manifest: dict) -> set[str]:
    """Every crate a manifest names, under its own key or a `package = ` rename."""
    tables = [manifest.get(table, {}) for table in DEPENDENCY_TABLES]
    tables.append(manifest.get("workspace", {}).get("dependencies", {}))
    for target in manifest.get("target", {}).values():
        tables.extend(target.get(table, {}) for table in DEPENDENCY_TABLES)
    names: set[str] = set()
    for table in tables:
        for key, value in table.items():
            names.add(key)
            if isinstance(value, dict) and isinstance(value.get("package"), str):
                names.add(value["package"])
    return names


def cargo_manifest_errors(label: str, manifest_path: pathlib.Path) -> list[str]:
    """Return why a first-party Cargo.toml drifts from the shared toolchain, if it does.

    Every first-party crate is on Rust edition 2024 (vendored crates follow
    their upstream and are not checked) and stays off the YAML crates the
    repo moved away from.
    """
    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        return [f"{label}: could not parse {manifest_path.name}: {error}"]
    errors = []
    package = manifest.get("package")
    if isinstance(package, dict) and "edition" not in package:
        errors.append(f'{label}: [package] must declare edition = "{RUST_EDITION}"')
    tables = (("package", package), ("workspace.package", manifest.get("workspace", {}).get("package")))
    for table_name, table in tables:
        edition = table.get("edition") if isinstance(table, dict) else None
        # `edition.workspace = true` inherits [workspace.package], checked there.
        if edition is None or edition == {"workspace": True}:
            continue
        if edition != RUST_EDITION:
            errors.append(f'{label}: [{table_name}] edition = {edition!r}, must be "{RUST_EDITION}"')
    for name in sorted(_dependency_names(manifest) & DENIED_CRATES.keys()):
        errors.append(f"{label}: depends on {name}: {DENIED_CRATES[name]}")
    return errors


def root_package_manager(root: pathlib.Path) -> str | None:
    """The repo-root package.json `packageManager`, the pin every package follows."""
    try:
        return json.loads((root / "package.json").read_text(encoding="utf-8")).get("packageManager")
    except (OSError, json.JSONDecodeError):
        return None


def package_manager_error(label: str, package_json: pathlib.Path, expected: str) -> str | None:
    """Return why a package.json pins another package manager than the repo root, or None.

    A package with its own pnpm-lock.yaml is installed on its own, so it must
    pin one; a workspace member without a pin uses the root's.
    """
    try:
        package = json.loads(package_json.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return f"{label}: could not parse {package_json.name}: {error}"
    pinned = package.get("packageManager")
    if pinned is None and not (package_json.parent / "pnpm-lock.yaml").exists():
        return None
    if pinned != expected:
        return f'{label}: packageManager is {pinned!r}, must be "{expected}" (the repo-root package.json pin)'
    return None


def validate_public_manifest(worker: str, spec: _lib.WorkerSpec, hard) -> None:
    """Validate the public `iii.worker.yaml` manifest while keeping it out of release planning."""
    try:
        manifest = _lib.read_iii_worker_yaml(spec.path)
    except (FileNotFoundError, ValueError) as error:
        hard(str(error))
        return

    for key in ("name", "language", "deploy", "manifest"):
        if not manifest.raw.get(key):
            hard(f"{worker}/iii.worker.yaml is missing key: {key}")
    if manifest.name != worker:
        hard(f"{worker}/iii.worker.yaml name={manifest.name!r} does not match folder")
    if manifest.language not in {"rust", "go", "node", "python", "javascript"}:
        hard(f"{worker}/iii.worker.yaml has unsupported language={manifest.language!r}")
    if manifest.deploy not in {"binary", "image", "bundle"}:
        hard(f"{worker}/iii.worker.yaml has unsupported deploy={manifest.deploy!r}")
    if manifest.manifest != spec.manifest:
        hard(
            f"{worker}/iii.worker.yaml manifest={manifest.manifest!r} must match "
            f".deploy/workers.yaml source.package_manifest={spec.manifest!r}"
        )

    expected_kinds = {
        "binary": {"rust-binary", "go-binary"},
        "image": {"oci-image"},
        "bundle": {"javascript-bundle", "python-bundle"},
    }
    if manifest.deploy in expected_kinds and spec.artifact_kind not in expected_kinds[manifest.deploy]:
        hard(
            f"{worker}/iii.worker.yaml deploy={manifest.deploy!r} does not match "
            f".deploy/workers.yaml artifact.kind={spec.artifact_kind!r}"
        )

    dependencies = manifest.raw.get("dependencies") or {}
    if not isinstance(dependencies, dict) or not all(
        isinstance(name, str) and isinstance(version, str)
        for name, version in dependencies.items()
    ):
        hard(f"{worker}/iii.worker.yaml dependencies must be a string-to-string mapping")

    iii_engine_version = manifest.raw.get("iii_engine_version")
    if "iii_engine_version" in manifest.raw and not (
        isinstance(iii_engine_version, str) and _lib.III_ENGINE_VERSION_RE.fullmatch(iii_engine_version)
    ):
        hard(
            f"{worker}/iii.worker.yaml iii_engine_version must be a semver range like "
            '">=0.26.0" (comparators >=, >, <=, <, =, ^, ~ on MAJOR.MINOR.PATCH, '
            f"joined by spaces or ||), got {iii_engine_version!r}"
        )

    if "interface_smoke" in manifest.raw:
        hard(f"{worker}/iii.worker.yaml interface_smoke was replaced by registry_interface")
    registry_interface = manifest.raw.get("registry_interface", True)
    if not isinstance(registry_interface, bool):
        hard(f"{worker}/iii.worker.yaml registry_interface must be a boolean when present")
    tags = manifest.raw.get("tags")
    if tags is not None and (
        not isinstance(tags, list) or not all(isinstance(tag, str) for tag in tags)
    ):
        hard(f"{worker}/iii.worker.yaml tags must be a string array")
    elif registry_interface and not tags:
        hard(f"{worker}/iii.worker.yaml tags must be non-empty when Registry interface publication is enabled")

    if manifest.deploy == "binary":
        executable = manifest.bin or manifest.name
        if executable != worker and worker not in BINARY_NAME_EXCEPTIONS:
            hard(f"{worker}/iii.worker.yaml bin={executable!r} must match the worker ID")
    if manifest.deploy == "bundle":
        scripts = manifest.raw.get("scripts") or {}
        if not isinstance(scripts, dict):
            hard(f"{worker}/iii.worker.yaml scripts must be a mapping")
        else:
            if str(scripts.get("setup") or "").strip():
                hard(f"{worker}/iii.worker.yaml bundle workers must not declare scripts.setup")
            if not str(scripts.get("start") or "").strip():
                hard(f"{worker}/iii.worker.yaml bundle workers must declare scripts.start")
        runtime = manifest.raw.get("runtime") or {}
        if not isinstance(runtime, dict):
            hard(f"{worker}/iii.worker.yaml runtime must be a mapping")
        else:
            base_image = runtime.get("base_image")
            if base_image is not None and base_image not in BUNDLE_PRESET_IMAGES:
                hard(
                    f"{worker}/iii.worker.yaml runtime.base_image must be one of "
                    f"{sorted(BUNDLE_PRESET_IMAGES)}"
                )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True)
    parser.add_argument("--base-ref", required=True)
    parser.add_argument("--source-changed", required=True)
    args = parser.parse_args(argv)

    worker = args.worker
    strict = worker in set(json.loads(args.source_changed))
    errors: list[str] = []

    def hard(message: str) -> None:
        errors.append(message)

    def soft(message: str) -> None:
        if strict:
            errors.append(message)
        else:
            print(f"::notice::{message} (skipped: {worker} only changed metadata)")

    try:
        spec = _lib.read_worker(worker)
    except (FileNotFoundError, ValueError) as error:
        hard(str(error))
        spec = None

    if spec is not None:
        validate_public_manifest(worker, spec, hard)
        if not spec.publish:
            hard(f"{worker}: first-party CI workers must set publish=true")
        expected_path = pathlib.Path(worker).resolve()
        if spec.path != expected_path:
            hard(f"{worker}: source.path must be {worker!r}, got {spec.source.get('path')!r}")
        # The descriptor compiler runs these same checks on main; a list the
        # Registry would refuse fails the whole descriptor index there.
        try:
            worker_screenshots.collect(spec.path, root=spec.path.parent, kind=spec.artifact_kind, worker=worker)
        except worker_screenshots.ScreenshotError as error:
            hard(str(error))
        readme = spec.path / "README.md"
        if not readme.exists():
            soft(f"{worker}/README.md is missing")
        elif readme.stat().st_size == 0:
            soft(f"{worker}/README.md is empty")

        tags = spec.registry.get("tags")
        if tags is not None and (
            not isinstance(tags, list) or any(not isinstance(tag, str) for tag in tags)
        ):
            hard(f"{worker}/iii.worker.yaml tags must be an array of strings")
        elif spec.validation.get("interface") == "required" and not tags:
            hard(f"{worker}/iii.worker.yaml tags must be non-empty when the PR interface smoke is enabled")

        if spec.artifact_kind in {"rust-binary", "go-binary"}:
            executable = spec.binary or worker
            if executable != worker and worker not in BINARY_NAME_EXCEPTIONS:
                hard(
                    f"{worker}: artifact.binary={executable!r} must match the worker ID "
                    "because Registry extraction addresses the worker by ID"
                )
        if spec.manifest:
            manifest_path = spec.path / spec.manifest
            if not manifest_path.is_file():
                hard(f"{manifest_path} not found")
            else:
                if spec.artifact_kind == "rust-binary" and manifest_path.name == "Cargo.toml":
                    profile_error = release_profile_error(worker, manifest_path)
                    if profile_error:
                        hard(profile_error)
                if manifest_path.name == "Cargo.toml":
                    for manifest_error in cargo_manifest_errors(f"{worker}/{spec.manifest}", manifest_path):
                        hard(manifest_error)
                root_pin = root_package_manager(spec.path.parent)
                if manifest_path.name == "package.json" and root_pin:
                    pin_error = package_manager_error(f"{worker}/{spec.manifest}", manifest_path, root_pin)
                    if pin_error:
                        hard(pin_error)
                try:
                    current_version = _lib.read_version(manifest_path)
                except (ValueError, FileNotFoundError) as error:
                    hard(f"could not read version from {manifest_path}: {error}")
                    current_version = None
                try:
                    base_blob = subprocess.check_output(
                        ["git", "show", f"{args.base_ref}:{worker}/{spec.manifest}"],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    )
                    base_sha = subprocess.check_output(
                        ["git", "rev-parse", args.base_ref],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                    head_sha = subprocess.check_output(
                        ["git", "rev-parse", "HEAD"],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                except subprocess.CalledProcessError:
                    base_blob = None
                    base_sha = head_sha = ""
                if current_version and base_blob is not None and base_sha != head_sha:
                    with tempfile.TemporaryDirectory() as directory:
                        temporary = pathlib.Path(directory) / spec.manifest
                        temporary.write_text(base_blob, encoding="utf-8")
                        try:
                            base_version = _lib.read_version(temporary)
                        except (ValueError, FileNotFoundError):
                            base_version = None
                    if base_version and _lib.parse_semver(current_version) < _lib.parse_semver(base_version):
                        soft(
                            f"{worker}/{spec.manifest} version {current_version} "
                            f"is less than base {base_version}"
                        )

        tests = spec.path / "tests"
        go_tests = spec.artifact_kind == "go-binary" and any(spec.path.rglob("*_test.go"))
        if not go_tests and not tests.exists():
            soft(f"{worker}/tests/ is missing")
        elif not go_tests and not any(tests.iterdir()):
            soft(f"{worker}/tests/ is empty")

    for error in errors:
        print(f"::error::{error}")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
