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


class TestSyncLockSubcommand:
    def _lock(self, dir_path: Path, name: str, version: str) -> Path:
        p = dir_path / "Cargo.lock"
        p.write_text(
            'version = 3\n\n'
            '[[package]]\n'
            'name = "leftpad"\n'
            'version = "1.0.0"\n\n'
            '[[package]]\n'
            f'name = "{name}"\n'
            f'version = "{version}"\n'
            'dependencies = [\n'
            ' "leftpad",\n'
            ']\n'
        )
        return p

    def test_syncs_stale_self_version(self, cargo_manifest):
        # cargo_manifest is name="smoke" version="0.1.0"; lock is stale at 0.0.9.
        lock = self._lock(cargo_manifest.parent, "smoke", "0.0.9")
        r = run_script("sync-lock", str(cargo_manifest))
        assert r.returncode == 0, r.stderr
        body = lock.read_text()
        assert 'name = "smoke"\nversion = "0.1.0"' in body
        # The unrelated dependency entry is untouched.
        assert 'name = "leftpad"\nversion = "1.0.0"' in body

    def test_idempotent_when_already_synced(self, cargo_manifest):
        lock = self._lock(cargo_manifest.parent, "smoke", "0.1.0")
        before = lock.read_text()
        r = run_script("sync-lock", str(cargo_manifest))
        assert r.returncode == 0
        assert "already in sync" in r.stdout
        assert lock.read_text() == before

    def test_noop_without_lockfile(self, cargo_manifest):
        r = run_script("sync-lock", str(cargo_manifest))
        assert r.returncode == 0  # no Cargo.lock present -> nothing to do

    def test_noop_for_non_cargo_manifest(self, package_json_manifest):
        r = run_script("sync-lock", str(package_json_manifest))
        assert r.returncode == 0


class TestDeployModeSubcommand:
    def test_binary_yields_release_binary(self, tmp_path):
        (tmp_path / "iii.worker.yaml").write_text(
            'iii: v1\nname: x\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
        )
        r = run_script("deploy-mode", str(tmp_path))
        assert r.returncode == 0
        assert r.stdout.strip() == "release-binary"

    def test_image_with_runtime_yields_iii_add(self, tmp_path):
        (tmp_path / "iii.worker.yaml").write_text(
            'iii: v1\nname: x\nlanguage: node\ndeploy: image\nmanifest: package.json\n'
            'runtime:\n  kind: node\n'
        )
        r = run_script("deploy-mode", str(tmp_path))
        assert r.returncode == 0
        assert r.stdout.strip() == "iii-add"

    def test_image_with_scripts_start_yields_iii_add(self, tmp_path):
        (tmp_path / "iii.worker.yaml").write_text(
            'iii: v1\nname: x\nlanguage: python\ndeploy: image\nmanifest: pyproject.toml\n'
            'scripts:\n  start: python -m smoke\n'
        )
        r = run_script("deploy-mode", str(tmp_path))
        assert r.returncode == 0
        assert r.stdout.strip() == "iii-add"

    def test_rust_no_runtime_yields_cargo_run(self, tmp_path):
        (tmp_path / "iii.worker.yaml").write_text(
            'iii: v1\nname: x\nlanguage: rust\ndeploy: image\nmanifest: Cargo.toml\n'
        )
        r = run_script("deploy-mode", str(tmp_path))
        assert r.returncode == 0
        assert r.stdout.strip() == "cargo-run"

    def test_missing_iii_worker_yaml(self, tmp_path):
        r = run_script("deploy-mode", str(tmp_path))
        assert r.returncode != 0
