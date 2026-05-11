"""Tests for the --assert-non-empty flag on collect_worker_interface.py."""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "collect_worker_interface.py"


def write_payload(path: Path, *, functions: list) -> None:
    path.write_text(json.dumps({"functions": functions, "triggers": []}))


class TestAssertNonEmpty:
    def test_passes_when_functions_non_empty(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[{"id": "smoke::ping"}])
        r = subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-non-empty", "--assert-file", str(out)],
            capture_output=True, text=True,
        )
        assert r.returncode == 0, r.stderr

    def test_fails_when_functions_empty(self, tmp_path):
        out = tmp_path / "interface.json"
        write_payload(out, functions=[])
        r = subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-non-empty", "--assert-file", str(out)],
            capture_output=True, text=True,
        )
        assert r.returncode != 0
        assert "empty" in (r.stderr + r.stdout).lower()

    def test_fails_when_functions_key_missing(self, tmp_path):
        out = tmp_path / "interface.json"
        out.write_text("{}")
        r = subprocess.run(
            [sys.executable, str(SCRIPT),
             "--assert-non-empty", "--assert-file", str(out)],
            capture_output=True, text=True,
        )
        assert r.returncode != 0
