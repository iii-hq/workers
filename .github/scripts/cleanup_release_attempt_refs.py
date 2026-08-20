#!/usr/bin/env python3
"""Remove terminal release-attempt refs after their evidence retention window.

Only refs below the exact release-control namespace are eligible. A missing or
unreadable commit timestamp is treated as unsafe and leaves the ref in place;
the Release Control ledger and GitHub artifacts remain the source of evidence.
"""

from __future__ import annotations

import argparse
import subprocess
from datetime import datetime, timedelta, timezone
from urllib.parse import quote


PREFIXES = ("heads/release-control", "tags/release-control")
ALLOWED_REF_PREFIXES = ("refs/heads/release-control/", "refs/tags/release-control/")


def gh_lines(*args: str) -> list[str]:
    output = subprocess.check_output(["gh", "api", *args], text=True)
    return [line for line in output.splitlines() if line]


def candidate_refs(repository: str) -> list[str]:
    refs: list[str] = []
    for prefix in PREFIXES:
        try:
            refs.extend(gh_lines("--paginate", "--jq", ".[].ref", f"repos/{repository}/git/matching-refs/{prefix}"))
        except subprocess.CalledProcessError as error:
            print(f"warning: could not list {prefix}: {error}")
    return sorted(set(refs))


def ref_commit_date(repository: str, ref: str) -> datetime | None:
    endpoint = f"repos/{repository}/commits?sha={quote(ref, safe='')}&per_page=1"
    try:
        values = gh_lines("--jq", ".[0].commit.committer.date // empty", endpoint)
    except subprocess.CalledProcessError as error:
        print(f"warning: could not inspect {ref}: {error}")
        return None
    if not values:
        return None
    try:
        return datetime.fromisoformat(values[0].replace("Z", "+00:00")).astimezone(timezone.utc)
    except ValueError:
        return None


def delete_ref(repository: str, ref: str) -> None:
    if not ref.startswith(ALLOWED_REF_PREFIXES):
        raise ValueError(f"ref outside cleanup namespace: {ref}")
    subprocess.run(["gh", "api", "--method", "DELETE", f"repos/{repository}/git/refs/{ref.removeprefix('refs/')}"], check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True)
    parser.add_argument("--older-than-days", type=int, default=7)
    args = parser.parse_args()
    if args.older_than_days < 1:
        parser.error("--older-than-days must be positive")

    cutoff = datetime.now(timezone.utc) - timedelta(days=args.older_than_days)
    removed = 0
    for ref in candidate_refs(args.repository):
        if not ref.startswith(ALLOWED_REF_PREFIXES):
            print(f"preserve unexpected ref: {ref}")
            continue
        updated_at = ref_commit_date(args.repository, ref)
        if updated_at is None or updated_at >= cutoff:
            print(f"preserve recent or unverifiable ref: {ref}")
            continue
        print(f"delete terminal release-attempt ref: {ref} (last commit {updated_at.isoformat()})")
        delete_ref(args.repository, ref)
        removed += 1
    print(f"removed {removed} release-attempt refs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
