"""Tests for first-run benchmark history branch creation."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from _test_helpers import GIT_HERMETIC_ENV
from bootstrap_benchmark_history import (
    BootstrapError,
    ensure_history_branch,
)


def git(repository: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=repository,
        check=True,
        env=GIT_HERMETIC_ENV,
        capture_output=True,
        text=True,
    ).stdout.strip()


def repository_with_bare_remote(tmp_path: Path) -> tuple[Path, Path]:
    remote = tmp_path / "remote.git"
    checkout = tmp_path / "checkout"
    git(tmp_path, "init", "--bare", "-q", str(remote))
    git(tmp_path, "init", "-q", "-b", "main", str(checkout))
    git(checkout, "config", "user.email", "test@example.com")
    git(checkout, "config", "user.name", "Test")
    (checkout / "README.md").write_text("main\n")
    git(checkout, "add", "README.md")
    git(checkout, "commit", "-q", "-m", "Initial main")
    git(checkout, "remote", "add", "origin", str(remote))
    git(checkout, "push", "-q", "-u", "origin", "main")
    return checkout, remote


def test_creates_empty_remote_history_branch_idempotently(tmp_path: Path) -> None:
    checkout, remote = repository_with_bare_remote(tmp_path)

    assert ensure_history_branch(checkout) is True
    first_head = git(remote, "rev-parse", "refs/heads/gh-pages")
    assert git(remote, "ls-tree", "--name-only", first_head) == ""

    assert ensure_history_branch(checkout) is False
    assert git(remote, "rev-parse", "refs/heads/gh-pages") == first_head


def test_rejects_an_invalid_history_branch_name(tmp_path: Path) -> None:
    checkout, _ = repository_with_bare_remote(tmp_path)

    with pytest.raises(BootstrapError, match="check-ref-format"):
        ensure_history_branch(checkout, branch="../invalid")
