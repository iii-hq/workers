from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import yaml


SCRIPT = Path(__file__).parents[1] / "verify_registry_lock.py"


def run_verify(tmp_path: Path, versions: dict[str, str]) -> subprocess.CompletedProcess[str]:
    lock = tmp_path / "iii.lock"
    lock.write_text(
        yaml.safe_dump(
            {
                "workers": {
                    "harness": {"version": "1.8.0"},
                    "state": {"version": "0.22.0"},
                }
            }
        )
    )
    return subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--lock",
            str(lock),
            "--worker",
            "harness",
            "--version",
            "1.8.0",
            "--expected-versions-json",
            json.dumps(versions),
        ],
        text=True,
        capture_output=True,
        check=False,
    )


def test_accepts_exact_stack_versions(tmp_path: Path):
    result = run_verify(tmp_path, {"harness": "1.8.0", "state": "0.22.0"})
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["resolved_versions"]["state"] == "0.22.0"


def test_rejects_stack_version_resolved_from_another_channel(tmp_path: Path):
    result = run_verify(tmp_path, {"harness": "1.8.0", "state": "0.23.0"})
    assert result.returncode != 0
    assert "state: expected 0.23.0, resolved 0.22.0" in result.stderr
