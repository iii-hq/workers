#!/usr/bin/env python3
"""Resolve auditable Harness E2E scenario profiles from the release catalog."""
from __future__ import annotations

import argparse
import hashlib
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

CATALOG_PATH = Path(".github/release-workers.yaml")
VALID_PROFILES = {"release", "custom", "full"}


@dataclass(frozen=True)
class HarnessE2eCatalog:
    required_profile: str
    scenarios: tuple[dict[str, str], ...]
    release_scenarios: tuple[str, ...]

    @property
    def ids(self) -> tuple[str, ...]:
        return tuple(entry["id"] for entry in self.scenarios)

    @property
    def profile_digest(self) -> str:
        canonical = json.dumps(
            {
                "required_profile": self.required_profile,
                "scenarios": list(self.ids),
                "required_scenarios": list(self.release_scenarios),
            },
            sort_keys=True,
            separators=(",", ":"),
        )
        return hashlib.sha256(canonical.encode()).hexdigest()


def _string_list(value: Any, field: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ValueError(f"{field} must be a non-empty array")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item:
            raise ValueError(f"{field} entries must be non-empty strings")
        if item in result:
            raise ValueError(f"{field} repeats {item}")
        result.append(item)
    return result


def load_profile_catalog(path: Path = CATALOG_PATH) -> HarnessE2eCatalog:
    raw = yaml.safe_load(path.read_text()) or {}
    release_control = raw.get("release_control") or {}
    if release_control.get("harness_e2e_profiles") != 1:
        raise ValueError("release catalog must expose harness_e2e_profiles: 1")
    config = raw.get("harness_e2e") or {}
    required_profile = config.get("required_profile")
    if required_profile != "release":
        raise ValueError("harness_e2e.required_profile must be release")

    raw_scenarios = config.get("scenarios")
    if not isinstance(raw_scenarios, list) or not raw_scenarios:
        raise ValueError("harness_e2e.scenarios must be a non-empty array")
    scenarios: list[dict[str, str]] = []
    seen: set[str] = set()
    for entry in raw_scenarios:
        if not isinstance(entry, dict):
            raise ValueError("harness_e2e.scenarios entries must be objects")
        scenario_id = entry.get("id")
        group = entry.get("group")
        if not isinstance(scenario_id, str) or not scenario_id:
            raise ValueError("Harness E2E scenario ids must be non-empty strings")
        if scenario_id in seen:
            raise ValueError(f"Harness E2E scenario id repeats {scenario_id}")
        if group not in {"Quality", "Operations", "Validation"}:
            raise ValueError(f"{scenario_id}: unsupported Harness E2E group")
        seen.add(scenario_id)
        scenarios.append({"id": scenario_id, "group": group})

    profiles = config.get("profiles") or {}
    release = profiles.get("release") or {}
    release_scenarios = _string_list(
        release.get("scenarios"), "harness_e2e.profiles.release.scenarios"
    )
    unknown = [scenario for scenario in release_scenarios if scenario not in seen]
    if unknown:
        raise ValueError(f"release profile references unknown scenarios: {', '.join(unknown)}")
    full = profiles.get("full") or {}
    if full.get("scenarios") != "all":
        raise ValueError("harness_e2e.profiles.full.scenarios must be all")
    return HarnessE2eCatalog(
        required_profile=required_profile,
        scenarios=tuple(scenarios),
        release_scenarios=tuple(release_scenarios),
    )


def parse_scenarios_json(value: str, field: str) -> list[str]:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as error:
        raise ValueError(f"{field} is not valid JSON: {error}") from error
    if not isinstance(parsed, list):
        raise ValueError(f"{field} must be a JSON array")
    result: list[str] = []
    for item in parsed:
        if not isinstance(item, str) or not item:
            raise ValueError(f"{field} entries must be non-empty strings")
        if item in result:
            raise ValueError(f"{field} repeats {item}")
        result.append(item)
    return result


def resolve_profile(
    catalog: HarnessE2eCatalog,
    *,
    available: list[str],
    profile: str,
    requested: list[str],
    catalog_sha: str,
    expected_catalog_sha: str = "",
) -> dict[str, Any]:
    if profile not in VALID_PROFILES:
        raise ValueError(f"validation_profile must be one of {sorted(VALID_PROFILES)}")
    if len(available) != len(set(available)) or not available:
        raise ValueError("code-defined scenarios must be non-empty and unique")
    if available != list(catalog.ids):
        raise ValueError("release catalog scenarios do not match harness-e2e list")
    if expected_catalog_sha and expected_catalog_sha != catalog_sha:
        raise ValueError(
            f"Harness E2E catalog moved from {expected_catalog_sha} to {catalog_sha}; create a new preview"
        )

    if profile == "custom":
        if not requested:
            raise ValueError("custom validation requires at least one scenario")
        selected = requested
    else:
        resolved = list(catalog.release_scenarios) if profile == "release" else available
        if requested and requested != resolved:
            raise ValueError(f"{profile} scenarios do not match the canonical profile")
        selected = resolved

    unknown = [scenario for scenario in selected if scenario not in available]
    if unknown:
        raise ValueError(f"unknown Harness E2E scenarios: {', '.join(unknown)}")
    required = list(catalog.release_scenarios)
    return {
        "validation_profile": profile,
        "scenarios": selected,
        "required_scenarios": required,
        "promotion_eligible": set(required).issubset(selected),
        "catalog_sha": catalog_sha,
        "profile_digest": catalog.profile_digest,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalog", type=Path, default=CATALOG_PATH)
    parser.add_argument("--available-json", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--scenarios-json", default="[]")
    parser.add_argument("--catalog-sha", required=True)
    parser.add_argument("--expected-catalog-sha", default="")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        result = resolve_profile(
            load_profile_catalog(args.catalog),
            available=parse_scenarios_json(args.available_json, "available scenarios"),
            profile=args.profile,
            requested=parse_scenarios_json(args.scenarios_json, "requested scenarios"),
            catalog_sha=args.catalog_sha,
            expected_catalog_sha=args.expected_catalog_sha,
        )
    except (FileNotFoundError, ValueError) as error:
        raise SystemExit(f"invalid Harness E2E profile: {error}") from error
    rendered = json.dumps(result, sort_keys=True)
    if args.output:
        args.output.write_text(rendered + "\n")
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
