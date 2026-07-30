"""Shared fixtures for .github/scripts/ test suite."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

# Make `.github/scripts/*.py` importable as top-level modules, and ensure
# the tests dir itself is on sys.path so sibling helper modules like
# `_test_helpers` are importable from both conftest and test_*.py.
SCRIPTS_DIR = Path(__file__).resolve().parents[1]
TESTS_DIR = Path(__file__).resolve().parent
for _p in (SCRIPTS_DIR, TESTS_DIR):
    if str(_p) not in sys.path:
        sys.path.insert(0, str(_p))

# Reuse this env in every fixture that shells out to `git`. Neutralising
# global/system config keeps `git tag -a` and `git commit` working on
# developer machines that have `commit.gpgsign=true` or other surprises in
# global config. Source-of-truth lives in `_test_helpers` so test modules
# can import the same constant without re-importing conftest.
from _test_helpers import GIT_HERMETIC_ENV  # noqa: E402


@pytest.fixture
def cargo_manifest(tmp_path: Path) -> Path:
    p = tmp_path / "Cargo.toml"
    p.write_text(
        '[package]\n'
        'name = "smoke"\n'
        'version = "0.1.0"\n'
        'edition = "2021"\n'
    )
    return p


@pytest.fixture
def package_json_manifest(tmp_path: Path) -> Path:
    p = tmp_path / "package.json"
    p.write_text('{\n  "name": "smoke",\n  "version": "0.1.0"\n}\n')
    return p


@pytest.fixture
def pyproject_manifest(tmp_path: Path) -> Path:
    p = tmp_path / "pyproject.toml"
    p.write_text(
        '[project]\n'
        'name = "smoke"\n'
        'version = "0.1.0"\n'
    )
    return p


@pytest.fixture
def iii_worker_yaml_dir(tmp_path: Path) -> Path:
    """Returns a tmp dir containing a minimal iii.worker.yaml (rust binary)."""
    (tmp_path / "iii.worker.yaml").write_text(
        'iii: v1\n'
        'name: smoke\n'
        'language: rust\n'
        'deploy: binary\n'
        'manifest: Cargo.toml\n'
        'bin: smoke-bin\n'
        'description: smoke test worker\n'
    )
    return tmp_path


@pytest.fixture
def github_output(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Captures writes to $GITHUB_OUTPUT into a tmp file the test can read."""
    f = tmp_path / "github_output"
    f.touch()
    monkeypatch.setenv("GITHUB_OUTPUT", str(f))
    return f


@pytest.fixture
def tmp_git_repo_with_tag(tmp_path: Path) -> Path:
    """Initialises a tmp git repo, makes one commit, creates an annotated tag
    `smoke/v0.1.0` with a multi-line body containing `registry-tag: next`.
    Used to test `read_tag_annotation` without monkeypatching subprocess."""
    subprocess.run(["git", "init", "-q", "-b", "main"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    (tmp_path / "README.md").write_text("hello\n")
    subprocess.run(["git", "add", "."], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    subprocess.run(["git", "commit", "-q", "-m", "init"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    subprocess.run(
        ["git", "tag", "-a", "smoke/v0.1.0", "-m",
         "Release smoke/v0.1.0\n\nregistry-tag: next\nother: value\n"],
        cwd=tmp_path,
        check=True,
        env=GIT_HERMETIC_ENV,
    )
    return tmp_path
