"""Tests for .github/scripts/validate_worker.py."""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from _test_helpers import GIT_HERMETIC_ENV

SCRIPT = Path(__file__).resolve().parents[1] / "validate_worker.py"


def make_worker(tmp_path: Path, name: str, version: str = "0.1.0",
                language: str = "rust", deploy: str = "binary",
                manifest_name: str = "Cargo.toml",
                tests_dir: bool = True) -> Path:
    """Returns the repo-root path; worker lives at <root>/<name>/."""
    w = tmp_path / name
    w.mkdir()
    (w / "README.md").write_text(f"# {name}\nhello\n")
    (w / "iii.worker.yaml").write_text(
        f'iii: v1\nname: {name}\nlanguage: {language}\n'
        f'deploy: {deploy}\nmanifest: {manifest_name}\n'
    )
    if manifest_name == "Cargo.toml":
        (w / manifest_name).write_text(f'[package]\nname = "{name}"\nversion = "{version}"\n')
    elif manifest_name == "package.json":
        (w / manifest_name).write_text(f'{{"name":"{name}","version":"{version}"}}')
    elif manifest_name == "pyproject.toml":
        (w / manifest_name).write_text(f'[project]\nname = "{name}"\nversion = "{version}"\n')
    if tests_dir:
        (w / "tests").mkdir()
        (w / "tests" / "smoke.rs").write_text("// stub\n")
    return tmp_path


def run_script(repo: Path, worker: str, base_ref: str = "main",
               source_changed: list[str] | None = None) -> subprocess.CompletedProcess[str]:
    src = json.dumps(source_changed or [])
    return subprocess.run(
        [sys.executable, str(SCRIPT),
         "--worker", worker, "--base-ref", base_ref, "--source-changed", src],
        capture_output=True,
        text=True,
        cwd=repo,
        env=GIT_HERMETIC_ENV,
    )


def init_git(repo: Path) -> None:
    def run(*args):
        return subprocess.run(args, cwd=repo, check=True, env=GIT_HERMETIC_ENV)
    run("git", "init", "-q", "-b", "main")
    run("git", "config", "user.email", "t@e.com")
    run("git", "config", "user.name", "T")
    run("git", "add", ".")
    run("git", "commit", "-q", "-m", "init")


class TestValidateWorker:
    def test_happy_path_passes(self, tmp_path):
        repo = make_worker(tmp_path, "smoke")
        init_git(repo)
        r = run_script(repo, "smoke", source_changed=["smoke"])
        assert r.returncode == 0, r.stderr

    def test_missing_readme_fails_in_strict_mode(self, tmp_path):
        repo = make_worker(tmp_path, "smoke")
        (repo / "smoke" / "README.md").unlink()
        init_git(repo)
        r = run_script(repo, "smoke", source_changed=["smoke"])
        assert r.returncode != 0
        assert "README" in r.stdout + r.stderr

    def test_missing_readme_passes_in_metadata_only_mode(self, tmp_path):
        repo = make_worker(tmp_path, "smoke")
        (repo / "smoke" / "README.md").unlink()
        init_git(repo)
        # source_changed=[] means metadata-only — README requirement is a notice.
        r = run_script(repo, "smoke", source_changed=[])
        assert r.returncode == 0

    def test_missing_tests_dir_fails_strict(self, tmp_path):
        repo = make_worker(tmp_path, "smoke", tests_dir=False)
        init_git(repo)
        r = run_script(repo, "smoke", source_changed=["smoke"])
        assert r.returncode != 0
        assert "tests" in r.stdout + r.stderr

    def test_wrong_deploy_value_fails(self, tmp_path):
        repo = make_worker(tmp_path, "smoke", deploy="binary")
        init_git(repo)
        meta = repo / "smoke" / "iii.worker.yaml"
        meta.write_text(meta.read_text().replace("deploy: binary", "deploy: weird"))
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "bad"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "smoke", source_changed=["smoke"])
        assert r.returncode != 0
        assert "deploy" in r.stdout + r.stderr

    def test_binary_bin_name_mismatch_fails(self, tmp_path):
        # A binary worker whose `bin` differs from the worker name ships a
        # release archive the resolver can't find (web/v1.1.x regression).
        repo = make_worker(tmp_path, "smoke", deploy="binary")
        meta = repo / "smoke" / "iii.worker.yaml"
        meta.write_text(meta.read_text() + "bin: iii-smoke\n")
        init_git(repo)
        r = run_script(repo, "smoke", source_changed=["smoke"])
        assert r.returncode != 0, r.stdout + r.stderr
        assert "iii-smoke" in r.stdout + r.stderr
        assert "not found in archive" in r.stdout + r.stderr

    def test_binary_bin_name_exception_allowlisted_passes(self, tmp_path):
        # `acp` intentionally ships a user-facing `iii-acp` binary; the
        # allowlist exempts it from the bin==name gate.
        repo = make_worker(tmp_path, "acp", deploy="binary")
        meta = repo / "acp" / "iii.worker.yaml"
        meta.write_text(meta.read_text() + "bin: iii-acp\n")
        init_git(repo)
        r = run_script(repo, "acp", source_changed=["acp"])
        assert r.returncode == 0, r.stdout + r.stderr

    def test_missing_name_field_fails(self, tmp_path):
        repo = make_worker(tmp_path, "smoke")
        meta = repo / "smoke" / "iii.worker.yaml"
        # Strip the `name:` line — _lib falls back to folder name, so the
        # downstream m.name != worker check passes trivially. The gate must
        # still reject manifests missing the required key.
        meta.write_text("\n".join(
            line for line in meta.read_text().splitlines()
            if not line.startswith("name:")
        ) + "\n")
        init_git(repo)
        r = run_script(repo, "smoke", source_changed=["smoke"])
        assert r.returncode != 0, r.stdout + r.stderr
        assert "name" in r.stdout + r.stderr

    def test_version_equal_to_base_passes(self, tmp_path):
        repo = make_worker(tmp_path, "smoke", version="1.0.0")
        init_git(repo)
        # Touch source so worker is source_changed, but don't bump version.
        (repo / "smoke" / "src.rs").write_text("// touch\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "touch"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "smoke", base_ref="main~1", source_changed=["smoke"])
        assert r.returncode == 0, r.stdout + r.stderr

    def test_version_less_than_base_fails(self, tmp_path):
        repo = make_worker(tmp_path, "smoke", version="1.0.1")
        init_git(repo)
        cargo = repo / "smoke" / "Cargo.toml"
        cargo.write_text(
            cargo.read_text().replace('version = "1.0.1"', 'version = "1.0.0"'),
        )
        (repo / "smoke" / "src.rs").write_text("// touch\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "downgrade"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "smoke", base_ref="main~1", source_changed=["smoke"])
        assert r.returncode != 0
        assert "version" in r.stdout + r.stderr
        assert "less" in r.stdout + r.stderr
