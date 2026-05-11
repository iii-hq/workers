"""Tests for .github/scripts/manifest_version.py."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "manifest_version.py"


def run_script(*args: str) -> subprocess.CompletedProcess[str]:
    """Run manifest_version.py with arguments; capture stdout/stderr/exit."""
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
    )


class TestReadSubcommand:
    def test_read_cargo(self, cargo_manifest):
        r = run_script("read", str(cargo_manifest))
        assert r.returncode == 0
        assert r.stdout.strip() == "0.1.0"

    def test_read_node(self, package_json_manifest):
        r = run_script("read", str(package_json_manifest))
        assert r.returncode == 0
        assert r.stdout.strip() == "0.1.0"

    def test_read_python(self, pyproject_manifest):
        r = run_script("read", str(pyproject_manifest))
        assert r.returncode == 0
        assert r.stdout.strip() == "0.1.0"

    def test_read_missing_file(self, tmp_path):
        r = run_script("read", str(tmp_path / "nope.toml"))
        assert r.returncode != 0

    def test_read_unsupported_manifest(self, tmp_path):
        p = tmp_path / "Makefile"
        p.write_text("# nope")
        r = run_script("read", str(p))
        assert r.returncode != 0
        assert "unsupported" in (r.stderr + r.stdout).lower()


class TestBumpSubcommand:
    def test_bump_cargo_patch(self, cargo_manifest):
        r = run_script("bump", str(cargo_manifest), "--kind", "patch")
        assert r.returncode == 0
        assert r.stdout.strip() == "0.1.1"
        # File was actually written.
        r2 = run_script("read", str(cargo_manifest))
        assert r2.stdout.strip() == "0.1.1"

    def test_bump_node_minor(self, package_json_manifest):
        r = run_script("bump", str(package_json_manifest), "--kind", "minor")
        assert r.returncode == 0
        assert r.stdout.strip() == "0.2.0"

    def test_bump_python_major(self, pyproject_manifest):
        r = run_script("bump", str(pyproject_manifest), "--kind", "major")
        assert r.returncode == 0
        assert r.stdout.strip() == "1.0.0"

    def test_bump_rejects_unknown_kind(self, cargo_manifest):
        r = run_script("bump", str(cargo_manifest), "--kind", "weird")
        assert r.returncode != 0


class TestVerifySubcommand:
    def test_verify_match(self, cargo_manifest):
        r = run_script("verify", str(cargo_manifest), "--expected", "0.1.0")
        assert r.returncode == 0

    def test_verify_mismatch(self, cargo_manifest):
        r = run_script("verify", str(cargo_manifest), "--expected", "9.9.9")
        assert r.returncode != 0
        assert "0.1.0" in (r.stderr + r.stdout)
        assert "9.9.9" in (r.stderr + r.stdout)
