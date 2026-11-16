import json
import subprocess
from pathlib import Path

import pytest
from validate_worker import package_manager_error, root_package_manager

REPO_ROOT = Path(__file__).resolve().parents[3]


def _tracked_package_jsons() -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "--", "*package.json"], cwd=REPO_ROOT, check=True, capture_output=True, text=True
    ).stdout.split()
    # Vendored packages follow their upstream.
    return sorted(path for path in listed if "vendor/" not in path and "node_modules/" not in path)


@pytest.mark.parametrize("package_json", _tracked_package_jsons())
def test_every_package_pins_the_root_package_manager(package_json: str) -> None:
    expected = root_package_manager(REPO_ROOT)
    assert expected is not None
    assert package_manager_error(package_json, REPO_ROOT / package_json, expected) is None


def test_root_pins_pnpm() -> None:
    assert (root_package_manager(REPO_ROOT) or "").startswith("pnpm@")


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
