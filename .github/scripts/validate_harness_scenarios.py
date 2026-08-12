#!/usr/bin/env python3
"""Validate an exact scenario selection without owning release policy."""

from __future__ import annotations

import argparse
import json
import re


def string_list(raw: str, field: str) -> list[str]:
    value = json.loads(raw)
    if not isinstance(value, list) or not value:
        raise ValueError(f"{field} must be a non-empty JSON array")
    if any(not isinstance(item, str) or not item for item in value):
        raise ValueError(f"{field} must contain non-empty strings")
    if len(value) != len(set(value)):
        raise ValueError(f"{field} must be unique")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--available-json", required=True)
    parser.add_argument("--requested-json", required=True)
    parser.add_argument("--required-json", required=True)
    parser.add_argument("--profile", choices=("release", "custom", "full"), required=True)
    parser.add_argument("--policy-digest", required=True)
    parser.add_argument("--policy-version", required=True)
    args = parser.parse_args()
    try:
        available = string_list(args.available_json, "available scenarios")
        requested = string_list(args.requested_json, "requested scenarios")
        required = string_list(args.required_json, "required scenarios")
        unknown = [item for item in requested if item not in available]
        if unknown:
            raise ValueError(f"unknown scenarios: {', '.join(unknown)}")
        missing = [item for item in required if item not in requested]
        if missing:
            raise ValueError(f"required scenarios are not selected: {', '.join(missing)}")
        policy_digest = args.policy_digest
        policy_version = args.policy_version
        if not re.fullmatch(r"[0-9a-f]{64}", policy_digest):
            raise ValueError("policy digest must be a lowercase SHA-256")
        if not re.fullmatch(r"[1-9][0-9]*", policy_version):
            raise ValueError("policy version must be a positive integer")
    except (json.JSONDecodeError, ValueError) as error:
        raise SystemExit(str(error)) from error
    print(
        json.dumps(
            {
                "validation_profile": args.profile,
                "scenarios": requested,
                "required_scenarios": required,
                "profile_digest": policy_digest,
                "policy_version": int(policy_version),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
