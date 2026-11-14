import json
import subprocess
from pathlib import Path

import pytest
from validate_worker import cargo_manifest_errors, package_manager_error, root_package_manager

REPO_ROOT = Path(__file__).resolve().parents[3]


def _tracked(pattern: str) -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "--", pattern], cwd=REPO_ROOT, check=True, capture_output=True, text=True
    ).stdout.split()
    # Vendored crates and packages follow their upstream.
    return sorted(path for path in listed if "vendor/" not in path and "node_modules/" not in path)


@pytest.mark.parametrize("manifest", _tracked("*Cargo.toml"))
def test_every_first_party_crate_is_on_the_shared_toolchain(manifest: str) -> None:
    assert cargo_manifest_errors(manifest, REPO_ROOT / manifest) == []


@pytest.mark.parametrize("package_json", _tracked("*package.json"))
def test_every_package_pins_the_root_package_manager(package_json: str) -> None:
    assert package_manager_error(package_json, REPO_ROOT / package_json, root_package_manager(REPO_ROOT)) is None


def test_root_pins_pnpm() -> None:
    assert (root_package_manager(REPO_ROOT) or "").startswith("pnpm@")


def _cargo(tmp_path: Path, text: str) -> Path:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(text)
    return manifest


def test_edition_2021_is_reported(tmp_path: Path) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\nedition = "2021"\n')
    assert cargo_manifest_errors("demo", manifest) == ['demo: [package] edition = \'2021\', must be "2024"']


def test_missing_edition_is_reported(tmp_path: Path) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\n')
    assert cargo_manifest_errors("demo", manifest) == ['demo: [package] must declare edition = "2024"']


def test_workspace_edition_is_checked_and_inherited(tmp_path: Path) -> None:
    manifest = _cargo(
        tmp_path,
        '[workspace.package]\nedition = "2021"\n\n[package]\nname = "demo"\nedition.workspace = true\n',
    )
    assert cargo_manifest_errors("demo", manifest) == [
        'demo: [workspace.package] edition = \'2021\', must be "2024"'
    ]


def test_edition_2024_passes(tmp_path: Path) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\nedition = "2024"\n\n[dependencies]\nserde_yaml_ng = "0.10"\n')
    assert cargo_manifest_errors("demo", manifest) == []


@pytest.mark.parametrize(
    "dependency",
    [
        '[dependencies]\nserde_yaml = "0.9"\n',
        '[dev-dependencies]\nserde_yml = "0.0.12"\n',
        '[target.\'cfg(unix)\'.dependencies]\nserde_yaml = "0.9"\n',
        '[dependencies]\nyaml = { package = "serde_yaml", version = "0.9" }\n',
        '[workspace.dependencies]\nserde_yaml = "0.9"\n',
    ],
)
def test_denied_yaml_crates_are_reported(tmp_path: Path, dependency: str) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\nedition = "2024"\n\n' + dependency)
    errors = cargo_manifest_errors("demo", manifest)
    assert len(errors) == 1 and "use serde_yaml_ng" in errors[0]


def _package(tmp_path: Path, package: dict, lockfile: bool = False) -> Path:
    package_json = tmp_path / "package.json"
    package_json.write_text(json.dumps(package))
    if lockfile:
        (tmp_path / "pnpm-lock.yaml").write_text("lockfileVersion: '9.0'\n")
    return package_json


def test_other_pnpm_pin_is_reported(tmp_path: Path) -> None:
    package_json = _package(tmp_path, {"packageManager": "pnpm@10.18.2"})
    error = package_manager_error("demo", package_json, "pnpm@11.13.1")
    assert error is not None and "pnpm@11.13.1" in error


def test_standalone_package_without_a_pin_is_reported(tmp_path: Path) -> None:
    package_json = _package(tmp_path, {"name": "demo"}, lockfile=True)
    assert package_manager_error("demo", package_json, "pnpm@11.13.1") is not None


def test_workspace_member_without_a_pin_passes(tmp_path: Path) -> None:
    package_json = _package(tmp_path, {"name": "demo"})
    assert package_manager_error("demo", package_json, "pnpm@11.13.1") is None


def test_root_pin_passes(tmp_path: Path) -> None:
    package_json = _package(tmp_path, {"packageManager": "pnpm@11.13.1"}, lockfile=True)
    assert package_manager_error("demo", package_json, "pnpm@11.13.1") is None
