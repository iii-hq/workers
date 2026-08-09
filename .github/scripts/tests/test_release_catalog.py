from __future__ import annotations

from pathlib import Path

import pytest
import yaml

from release_catalog import load_catalog, resolved_entries, validate_checkout
from harness_e2e_profiles import load_profile_catalog


def test_repository_catalog_is_valid():
    catalog = load_catalog()
    validate_checkout(catalog)
    assert catalog["harness"]["allow_direct_latest"] is False
    assert catalog["harness"]["required_validation"] == "full"
    profiles = load_profile_catalog()
    assert profiles.required_profile == "release"
    assert len(profiles.ids) == 16
    assert len(profiles.release_scenarios) == 9
    assert catalog["lsp-vscode"]["release_workflow"] == "release-lsp-vscode.yml"
    resolved = {entry["slug"]: entry for entry in resolved_entries(catalog)}
    assert resolved["harness"]["manifest"] == "Cargo.toml"
    assert resolved["lsp-vscode"]["manifest"] == "package.json"


def test_rejects_policy_for_unknown_worker(tmp_path: Path):
    catalog = tmp_path / "release-workers.yaml"
    catalog.write_text(
        yaml.safe_dump(
            {
                "schema_version": 1,
                "defaults": {
                    "release_workflow": "release.yml",
                    "allow_direct_latest": True,
                    "required_validation": "smoke",
                    "release_control_enabled": True,
                },
                "standard_workers": [],
                "special_workers": {},
                "policies": {"missing": {"required_validation": "full"}},
            }
        )
    )
    with pytest.raises(ValueError, match="unknown release worker"):
        load_catalog(catalog)
