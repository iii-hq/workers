import subprocess
from pathlib import Path

import pytest
from validate_worker import edition_errors

REPO_ROOT = Path(__file__).resolve().parents[3]


def _tracked_cargo_manifests() -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "--", "*Cargo.toml"], cwd=REPO_ROOT, check=True, capture_output=True, text=True
    ).stdout.split()
    # Vendored crates follow their upstream.
    return sorted(path for path in listed if "vendor/" not in path)


@pytest.mark.parametrize("manifest", _tracked_cargo_manifests())
def test_every_first_party_crate_is_on_edition_2024(manifest: str) -> None:
    assert edition_errors(manifest, REPO_ROOT / manifest) == []


def _cargo(tmp_path: Path, text: str) -> Path:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(text)
    return manifest


def test_edition_2021_is_reported(tmp_path: Path) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\nedition = "2021"\n')
    assert edition_errors("demo", manifest) == ['demo: [package] edition = \'2021\', must be "2024"']


def test_missing_edition_is_reported(tmp_path: Path) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\n')
    assert edition_errors("demo", manifest) == ['demo: [package] must declare edition = "2024"']


def test_workspace_edition_is_checked_and_inherited(tmp_path: Path) -> None:
    manifest = _cargo(
        tmp_path,
        '[workspace.package]\nedition = "2021"\n\n[package]\nname = "demo"\nedition.workspace = true\n',
    )
    assert edition_errors("demo", manifest) == ['demo: [workspace.package] edition = \'2021\', must be "2024"']


def test_edition_2024_passes(tmp_path: Path) -> None:
    manifest = _cargo(tmp_path, '[package]\nname = "demo"\nedition = "2024"\n')
    assert edition_errors("demo", manifest) == []
