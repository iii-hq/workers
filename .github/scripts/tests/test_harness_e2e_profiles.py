from __future__ import annotations

from pathlib import Path

import pytest

from harness_e2e_profiles import load_profile_catalog, resolve_profile


def test_repository_profiles_match_the_code_defined_catalog():
    catalog = load_profile_catalog()
    resolved = resolve_profile(
        catalog,
        available=list(catalog.ids),
        profile="release",
        requested=[],
        catalog_sha="a" * 40,
    )
    assert resolved["scenarios"] == [
        "persistent_state",
        "reactive_automation",
        "mechanical_reaction",
        "timer_wake",
        "receiving_operation",
        "validation_loop",
        "subagent_validation",
        "validation_scope_enforcement",
        "validation_chain",
    ]
    assert resolved["promotion_eligible"] is True
    assert len(resolved["profile_digest"]) == 64


def test_custom_subset_is_valid_but_not_promotable():
    catalog = load_profile_catalog()
    resolved = resolve_profile(
        catalog,
        available=list(catalog.ids),
        profile="custom",
        requested=["persistent_state"],
        catalog_sha="a" * 40,
    )
    assert resolved["scenarios"] == ["persistent_state"]
    assert resolved["promotion_eligible"] is False


def test_custom_superset_is_promotable():
    catalog = load_profile_catalog()
    selected = [*catalog.release_scenarios, "direct_answer"]
    resolved = resolve_profile(
        catalog,
        available=list(catalog.ids),
        profile="custom",
        requested=selected,
        catalog_sha="a" * 40,
    )
    assert resolved["promotion_eligible"] is True


def test_rejects_unknown_scenario_and_stale_catalog():
    catalog = load_profile_catalog()
    with pytest.raises(ValueError, match="unknown Harness E2E scenarios"):
        resolve_profile(
            catalog,
            available=list(catalog.ids),
            profile="custom",
            requested=["missing"],
            catalog_sha="a" * 40,
        )
    with pytest.raises(ValueError, match="catalog moved"):
        resolve_profile(
            catalog,
            available=list(catalog.ids),
            profile="release",
            requested=[],
            catalog_sha="a" * 40,
            expected_catalog_sha="b" * 40,
        )


def test_catalog_rejects_duplicate_scenarios(tmp_path: Path):
    path = tmp_path / "release-workers.yaml"
    path.write_text(
        """
release_control: { harness_e2e_profiles: 1 }
harness_e2e:
  required_profile: release
  scenarios:
    - { id: duplicate, group: Quality }
    - { id: duplicate, group: Quality }
  profiles:
    release: { scenarios: [duplicate] }
    full: { scenarios: all }
"""
    )
    with pytest.raises(ValueError, match="repeats duplicate"):
        load_profile_catalog(path)
