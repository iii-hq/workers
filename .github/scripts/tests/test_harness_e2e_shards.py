from __future__ import annotations

import pytest

from harness_e2e_shards import PILOT_SCENARIOS, execution_shards, parse_scenarios


ALL_SCENARIOS = [
    "direct_answer",
    "persistent_state",
    "security_review",
    "reactive_automation",
    "shell_coder_sandbox",
    "design_tradeoff",
    "security_triage",
    "research_pipeline",
    "mechanical_reaction",
    "timer_wake",
    "receiving_operation",
    "validation_loop",
    "subagent_validation",
    "multi_subagent_validation",
    "subagent_validation_failure",
    "custom_validator",
    "validation_self_repair",
    "validation_scope_enforcement",
    "validation_chain",
]


def test_stateless_pilot_reduces_full_suite_to_fifteen_shards() -> None:
    shards = execution_shards(ALL_SCENARIOS, "stateless-pilot")

    assert len(shards) == 15
    assert shards[0] == {
        "id": "stateless-core",
        "scenario_ids": list(PILOT_SCENARIOS),
        "requires_kvm": False,
    }
    assert [
        scenario for shard in shards for scenario in shard["scenario_ids"]
    ] == [*PILOT_SCENARIOS] + [
        scenario for scenario in ALL_SCENARIOS if scenario not in PILOT_SCENARIOS
    ]


def test_isolated_profile_keeps_one_scenario_per_shard() -> None:
    shards = execution_shards(ALL_SCENARIOS, "isolated")

    assert len(shards) == 19
    assert all(len(shard["scenario_ids"]) == 1 for shard in shards)
    sandbox = next(shard for shard in shards if shard["id"] == "shell_coder_sandbox")
    assert sandbox["requires_kvm"] is True


def test_custom_subset_groups_only_selected_pilot_scenarios() -> None:
    shards = execution_shards(
        ["security_review", "timer_wake", "direct_answer"], "stateless-pilot"
    )

    assert shards == [
        {
            "id": "stateless-core",
            "scenario_ids": ["direct_answer", "security_review"],
            "requires_kvm": False,
        },
        {
            "id": "timer_wake",
            "scenario_ids": ["timer_wake"],
            "requires_kvm": False,
        },
    ]


@pytest.mark.parametrize("raw", ['[]', '["direct_answer","direct_answer"]'])
def test_rejects_empty_or_duplicate_scenarios(raw: str) -> None:
    with pytest.raises(ValueError):
        parse_scenarios(raw)
