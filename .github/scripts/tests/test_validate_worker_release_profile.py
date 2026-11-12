from pathlib import Path

import pytest
import yaml

from validate_worker import release_profile_error

REPO_ROOT = Path(__file__).resolve().parents[3]


def _rust_workers() -> list[str]:
    catalog = yaml.safe_load((REPO_ROOT / ".deploy" / "workers.yaml").read_text())["workers"]
    return sorted(name for name, spec in catalog.items() if spec["artifact"]["kind"] == "rust-binary")


@pytest.mark.parametrize("worker", _rust_workers())
def test_every_rust_worker_declares_a_release_profile(worker: str) -> None:
    assert release_profile_error(worker, REPO_ROOT / worker / "Cargo.toml") is None


def test_missing_release_profile_is_reported(tmp_path: Path) -> None:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[package]\nname = "demo"\n\n[profile.dev]\nopt-level = 1\n')
    error = release_profile_error("demo", manifest)
    assert error is not None and "[profile.release]" in error


def test_declared_release_profile_passes(tmp_path: Path) -> None:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[package]\nname = "demo"\n\n[profile.release]\nlto = "thin"\n')
    assert release_profile_error("demo", manifest) is None
