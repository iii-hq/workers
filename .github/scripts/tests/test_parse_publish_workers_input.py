"""Tests for .github/scripts/parse_publish_workers_input.py."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "parse_publish_workers_input.py"

# Import after path is known.
sys.path.insert(0, str(SCRIPT.parent))
from parse_publish_workers_input import ALLOWED_WORKERS, parse_workers  # noqa: E402


class TestParseWorkers:
    def test_all_expands_to_full_list(self):
        assert parse_workers("all") == list(ALLOWED_WORKERS)
        assert parse_workers("  ALL  ") == list(ALLOWED_WORKERS)

    def test_comma_separated_list(self):
        assert parse_workers("shell,coder") == ["shell", "coder"]

    def test_dedupe_preserves_order(self):
        assert parse_workers("shell,coder,shell") == ["shell", "coder"]

    def test_whitespace_trimmed(self):
        assert parse_workers(" shell , coder ") == ["shell", "coder"]

    def test_unknown_worker_raises(self):
        with pytest.raises(ValueError, match="unknown worker"):
            parse_workers("shell,not-a-worker")

    def test_empty_raises(self):
        with pytest.raises(ValueError, match="empty"):
            parse_workers("")
        with pytest.raises(ValueError, match="empty"):
            parse_workers("  ,  , ")


def run_script(workers: str, github_output: Path) -> subprocess.CompletedProcess[str]:
    env = {**os.environ, "GITHUB_OUTPUT": str(github_output)}
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--workers", workers],
        capture_output=True,
        text=True,
        env=env,
    )


def test_cli_writes_matrix_to_github_output(tmp_path):
    out_file = tmp_path / "output"
    out_file.write_text("")
    result = run_script("shell,coder", out_file)
    assert result.returncode == 0
    lines = out_file.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 1
    key, value = lines[0].split("=", 1)
    assert key == "matrix"
    assert json.loads(value) == ["shell", "coder"]


def test_cli_unknown_worker_exits_nonzero():
    result = run_script("bogus", Path("/dev/null"))
    assert result.returncode == 1
    assert "unknown worker" in result.stderr
