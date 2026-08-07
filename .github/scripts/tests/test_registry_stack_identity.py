from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

import pytest
import yaml


SCRIPT = Path(__file__).parents[1] / "registry_stack_identity.py"


def run_identity(
    tmp_path: Path, workers: dict[str, dict[str, str]]
) -> subprocess.CompletedProcess[str]:
    lock = tmp_path / "iii.lock"
    lock.write_text(yaml.safe_dump({"workers": workers}, sort_keys=True))
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--lock", str(lock)],
        text=True,
        capture_output=True,
        check=False,
    )


def test_returns_exact_versions_and_lock_digest(tmp_path: Path) -> None:
    workers = {
        "harness": {"version": "1.8.0"},
        "state": {"version": "0.22.0"},
    }
    result = run_identity(tmp_path, workers)

    assert result.returncode == 0, result.stderr
    identity = json.loads(result.stdout)
    lock = tmp_path / "iii.lock"
    assert identity["stack_versions"] == {"harness": "1.8.0", "state": "0.22.0"}
    assert identity["lock_digest"] == hashlib.sha256(lock.read_bytes()).hexdigest()


@pytest.mark.parametrize(
    ("workers", "message"),
    [
        ({"state": {"version": "0.22.0"}}, "harness is required"),
        ({"harness": {"version": "latest"}}, "exact published version"),
    ],
)
def test_rejects_non_identity_locks(
    tmp_path: Path,
    workers: dict[str, dict[str, str]],
    message: str,
) -> None:
    result = run_identity(tmp_path, workers)

    assert result.returncode != 0
    assert message in result.stderr
