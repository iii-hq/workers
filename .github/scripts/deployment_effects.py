#!/usr/bin/env python3
"""Classify idempotent release effects around failures and retries."""

from __future__ import annotations

import argparse


STATES = {"absent", "present", "unknown"}
MUTATIONS = {"not_started", "started", "completed"}


def classify_effect(*, before: str, mutation: str, after: str) -> str:
    """Return the most precise state justified by pre/post probes.

    Mutations represented by this helper only create or promote an expected
    immutable identity; they never delete it.  A known-present precondition
    therefore remains present even if a later probe is unavailable.
    """
    if before not in STATES or after not in STATES:
        raise ValueError("effect probe state must be absent, present, or unknown")
    if mutation not in MUTATIONS:
        raise ValueError("mutation state must be not_started, started, or completed")
    if after != "unknown":
        return after
    if before == "present":
        return "present"
    if before == "absent" and mutation == "not_started":
        return "absent"
    return "unknown"


def mutation_plan(before: str) -> str:
    """Choose an idempotent retry action from an authoritative pre-probe."""
    if before == "present":
        return "skip"
    if before == "absent":
        return "mutate"
    if before == "unknown":
        return "refuse"
    raise ValueError("effect probe state must be absent, present, or unknown")


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    classify = subparsers.add_parser("classify")
    classify.add_argument("--before", choices=sorted(STATES), required=True)
    classify.add_argument("--mutation", choices=sorted(MUTATIONS), required=True)
    classify.add_argument("--after", choices=sorted(STATES), required=True)
    plan = subparsers.add_parser("plan")
    plan.add_argument("--before", choices=sorted(STATES), required=True)
    args = parser.parse_args()
    if args.command == "classify":
        print(classify_effect(before=args.before, mutation=args.mutation, after=args.after))
    else:
        print(mutation_plan(args.before))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
