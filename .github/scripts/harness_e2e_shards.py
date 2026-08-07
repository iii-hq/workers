#!/usr/bin/env python3
"""Create deterministic Harness E2E execution shards."""

from __future__ import annotations

import argparse
import json
import re
from typing import Any


PILOT_SCENARIOS = (
    "direct_answer",
    "security_review",
    "design_tradeoff",
    "security_triage",
    "custom_validator",
)
VALID_PROFILES = {"isolated", "stateless-pilot"}
SCENARIO_ID = re.compile(r"^[a-z0-9][a-z0-9_]*$")


def parse_scenarios(raw: str) -> list[str]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ValueError(f"scenarios are not valid JSON: {error}") from error
    if not isinstance(value, list) or not value:
        raise ValueError("scenarios must be a non-empty array")
    scenarios: list[str] = []
    for scenario in value:
        if not isinstance(scenario, str) or not SCENARIO_ID.fullmatch(scenario):
            raise ValueError(f"invalid scenario id: {scenario!r}")
        if scenario in scenarios:
            raise ValueError(f"scenario repeats: {scenario}")
        scenarios.append(scenario)
    return scenarios


def execution_shards(scenarios: list[str], profile: str) -> list[dict[str, Any]]:
    if profile not in VALID_PROFILES:
        raise ValueError(f"sharding profile must be one of {sorted(VALID_PROFILES)}")
    if not scenarios or len(scenarios) != len(set(scenarios)):
        raise ValueError("scenarios must be non-empty and unique")

    grouped: list[str] = []
    if profile == "stateless-pilot":
        grouped = [scenario for scenario in PILOT_SCENARIOS if scenario in scenarios]

    shards: list[dict[str, Any]] = []
    if len(grouped) > 1:
        shards.append(
            {
                "id": "stateless-core",
                "scenario_ids": grouped,
                "requires_kvm": False,
            }
        )
    else:
        grouped = []

    grouped_set = set(grouped)
    shards.extend(
        {
            "id": scenario,
            "scenario_ids": [scenario],
            "requires_kvm": scenario == "shell_coder_sandbox",
        }
        for scenario in scenarios
        if scenario not in grouped_set
    )

    flattened = [scenario for shard in shards for scenario in shard["scenario_ids"]]
    if set(flattened) != set(scenarios) or len(flattened) != len(scenarios):
        raise ValueError("shards must contain every scenario exactly once")
    return shards


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scenarios-json", required=True)
    parser.add_argument("--profile", default="isolated")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        shards = execution_shards(parse_scenarios(args.scenarios_json), args.profile)
    except ValueError as error:
        raise SystemExit(f"invalid_shards: {error}") from error
    print(json.dumps(shards, separators=(",", ":")))


if __name__ == "__main__":
    main()
