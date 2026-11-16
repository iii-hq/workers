import subprocess
from pathlib import Path

import pytest
from validate_worker import denied_crate_errors

REPO_ROOT = Path(__file__).resolve().parents[3]


def _tracked_cargo_manifests() -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "--", "*Cargo.toml"], cwd=REPO_ROOT, check=True, capture_output=True, text=True
    ).stdout.split()
    # Vendored crates follow their upstream.
    return sorted(path for path in listed if "vendor/" not in path)


@pytest.mark.parametrize("manifest", _tracked_cargo_manifests())
def test_no_first_party_crate_depends_on_a_denied_crate(manifest: str) -> None:
    assert denied_crate_errors(manifest, REPO_ROOT / manifest) == []


@pytest.mark.parametrize(
    "dependency",
    [
        '[dependencies]\nserde_yaml = "0.9"\n',
        '[dev-dependencies]\nserde_yml = "0.0.12"\n',
        '[build-dependencies]\nserde_yaml = "0.9"\n',
        "[target.'cfg(unix)'.dependencies]\nserde_yaml = \"0.9\"\n",
        '[dependencies]\nyaml = { package = "serde_yaml", version = "0.9" }\n',
        '[workspace.dependencies]\nserde_yaml = "0.9"\n',
    ],
)
def test_denied_yaml_crates_are_reported(tmp_path: Path, dependency: str) -> None:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[package]\nname = "demo"\n\n' + dependency)
    errors = denied_crate_errors("demo", manifest)
    assert len(errors) == 1 and "use serde_yaml_ng" in errors[0]


def test_serde_yaml_ng_passes(tmp_path: Path) -> None:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[package]\nname = "demo"\n\n[dependencies]\nserde_yaml_ng = "0.10"\n')
    assert denied_crate_errors("demo", manifest) == []
