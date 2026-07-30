#!/usr/bin/env python3
"""Create an empty remote benchmark-history branch when it does not exist."""

from __future__ import annotations

import argparse
import subprocess
from pathlib import Path


class BootstrapError(RuntimeError):
    """Raised when the benchmark history branch cannot be inspected or created."""


def run_git(
    repository: Path,
    *args: str,
    input_text: str | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["git", *args],
        cwd=repository,
        input=input_text,
        text=True,
        capture_output=True,
    )
    if check and result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise BootstrapError(f"git {' '.join(args)} failed: {detail}")
    return result


def remote_branch_exists(repository: Path, remote: str, branch: str) -> bool:
    ref = f"refs/heads/{branch}"
    run_git(repository, "check-ref-format", ref)
    result = run_git(
        repository,
        "ls-remote",
        "--exit-code",
        "--heads",
        remote,
        ref,
        check=False,
    )
    if result.returncode == 0:
        return True
    if result.returncode == 2:
        return False
    detail = result.stderr.strip() or result.stdout.strip()
    raise BootstrapError(f"cannot inspect {remote}/{branch}: {detail}")


def ensure_history_branch(
    repository: Path,
    *,
    remote: str = "origin",
    branch: str = "gh-pages",
    message: str = "Initialize benchmark history",
) -> bool:
    """Ensure the remote branch exists, returning true only when it was created."""
    if remote_branch_exists(repository, remote, branch):
        return False

    empty_tree = run_git(repository, "mktree", input_text="").stdout.strip()
    commit = run_git(
        repository,
        "-c",
        "user.name=github-actions",
        "-c",
        "user.email=github-actions@github.com",
        "commit-tree",
        empty_tree,
        input_text=f"{message}\n",
    ).stdout.strip()
    push = run_git(
        repository,
        "push",
        remote,
        f"{commit}:refs/heads/{branch}",
        check=False,
    )
    if push.returncode == 0:
        return True

    # Another publisher may have won the race after the initial lookup.
    if remote_branch_exists(repository, remote, branch):
        return False
    detail = push.stderr.strip() or push.stdout.strip()
    raise BootstrapError(f"cannot create {remote}/{branch}: {detail}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", type=Path, default=Path("."))
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--branch", default="gh-pages")
    parser.add_argument("--message", default="Initialize benchmark history")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    created = ensure_history_branch(
        args.repository.resolve(),
        remote=args.remote,
        branch=args.branch,
        message=args.message,
    )
    state = "created" if created else "already exists"
    print(f"{args.remote}/{args.branch} {state}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
