"""Tests for .github/scripts/discover_changed_workers.py."""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from _test_helpers import GIT_HERMETIC_ENV

SCRIPT = Path(__file__).resolve().parents[1] / "discover_changed_workers.py"


def make_repo_with_workers(tmp_path: Path) -> Path:
    """Initialise a tmp git repo with three workers (rust binary, node image,
    python image) and one non-worker dir. Returns the repo path."""
    def run(*args, **kwargs):
        return subprocess.run(args, cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV, **kwargs)
    run("git", "init", "-q", "-b", "main")
    run("git", "config", "user.email", "t@e.com")
    run("git", "config", "user.name", "T")
    # Worker A — rust binary
    (tmp_path / "worker-a").mkdir()
    (tmp_path / "worker-a" / "iii.worker.yaml").write_text(
        'iii: v1\nname: worker-a\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
    )
    (tmp_path / "worker-a" / "Cargo.toml").write_text(
        '[package]\nname = "worker-a"\nversion = "0.1.0"\n'
    )
    # Worker B — node image
    (tmp_path / "worker-b").mkdir()
    (tmp_path / "worker-b" / "iii.worker.yaml").write_text(
        'iii: v1\nname: worker-b\nlanguage: node\ndeploy: image\nmanifest: package.json\n'
    )
    (tmp_path / "worker-b" / "package.json").write_text('{"name":"worker-b","version":"0.1.0"}')
    # Worker C — python image
    (tmp_path / "worker-c").mkdir()
    (tmp_path / "worker-c" / "iii.worker.yaml").write_text(
        'iii: v1\nname: worker-c\nlanguage: python\ndeploy: image\nmanifest: pyproject.toml\n'
    )
    (tmp_path / "worker-c" / "pyproject.toml").write_text(
        '[project]\nname = "worker-c"\nversion = "0.1.0"\n'
    )
    # Non-worker dir
    (tmp_path / "docs").mkdir()
    (tmp_path / "docs" / "README.md").write_text("# hello\n")
    run("git", "add", ".")
    run("git", "commit", "-q", "-m", "init")
    return tmp_path


def run_script(repo: Path, base: str, head: str = "HEAD") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--base", base, "--head", head],
        capture_output=True,
        text=True,
        cwd=repo,
        env=GIT_HERMETIC_ENV,
    )


class TestDiscoverChangedWorkers:
    def test_single_worker_source_change(self, tmp_path):
        repo = make_repo_with_workers(tmp_path)
        (repo / "worker-a" / "src.rs").write_text("// hi\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "edit a"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["worker-a"]
        assert data["source_changed"] == ["worker-a"]
        assert data["by_language"]["rust"] == ["worker-a"]

    def test_multi_worker_change(self, tmp_path):
        repo = make_repo_with_workers(tmp_path)
        (repo / "worker-a" / "src.rs").write_text("// hi\n")
        (repo / "worker-c" / "x.py").write_text("# hi\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "edit a+c"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0
        data = json.loads(r.stdout)
        assert sorted(data["changed_workers"]) == ["worker-a", "worker-c"]
        assert sorted(data["by_language"]["rust"]) == ["worker-a"]
        assert sorted(data["by_language"]["python"]) == ["worker-c"]

    def test_metadata_only_change_not_source_changed(self, tmp_path):
        repo = make_repo_with_workers(tmp_path)
        (repo / "worker-a" / "README.md").write_text("# a\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "docs"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["worker-a"]
        assert data["source_changed"] == []

    def test_non_worker_change_ignored(self, tmp_path):
        repo = make_repo_with_workers(tmp_path)
        (repo / "docs" / "more.md").write_text("# more\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "doc"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0
        data = json.loads(r.stdout)
        assert data["changed_workers"] == []


def make_repo_with_harness(tmp_path: Path) -> Path:
    """Tmp repo: harness + two in-repo deps + one external dep + one unrelated worker."""
    def run(*args):
        return subprocess.run(args, cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    run("git", "init", "-q", "-b", "main")
    run("git", "config", "user.email", "t@e.com")
    run("git", "config", "user.name", "T")
    (tmp_path / "harness").mkdir()
    (tmp_path / "harness" / "iii.worker.yaml").write_text(
        'iii: v1\nname: harness\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
        'dependencies:\n'
        '  dep-rust: "^0.1.0"\n'
        '  dep-node: "^0.1.0"\n'
        '  external-dep: "^0.1.0"\n'
    )
    (tmp_path / "harness" / "Cargo.toml").write_text(
        '[package]\nname = "harness"\nversion = "0.1.0"\n'
    )
    (tmp_path / "dep-rust").mkdir()
    (tmp_path / "dep-rust" / "iii.worker.yaml").write_text(
        'iii: v1\nname: dep-rust\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
    )
    (tmp_path / "dep-rust" / "Cargo.toml").write_text(
        '[package]\nname = "dep-rust"\nversion = "0.1.0"\n'
    )
    (tmp_path / "dep-node").mkdir()
    (tmp_path / "dep-node" / "iii.worker.yaml").write_text(
        'iii: v1\nname: dep-node\nlanguage: node\ndeploy: image\nmanifest: package.json\n'
    )
    (tmp_path / "dep-node" / "package.json").write_text('{"name":"dep-node","version":"0.1.0"}')
    (tmp_path / "unrelated").mkdir()
    (tmp_path / "unrelated" / "iii.worker.yaml").write_text(
        'iii: v1\nname: unrelated\nlanguage: python\ndeploy: image\nmanifest: pyproject.toml\n'
    )
    (tmp_path / "unrelated" / "pyproject.toml").write_text(
        '[project]\nname = "unrelated"\nversion = "0.1.0"\n'
    )
    run("git", "add", ".")
    run("git", "commit", "-q", "-m", "init")
    return tmp_path


class TestHarnessFanOut:
    def test_harness_source_change_fans_out_to_in_repo_deps(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "lib.rs").write_text("// breaking change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert sorted(data["changed_workers"]) == ["dep-node", "dep-rust", "harness"]
        assert "unrelated" not in data["changed_workers"]
        # External dep listed in harness deps but absent from repo is skipped.
        assert "external-dep" not in data["changed_workers"]

    def test_fanned_out_deps_excluded_from_source_changed(self, tmp_path):
        """Fan-out enters deps into the matrix (so lint+test runs against the
        new harness) but must NOT mark them source_changed. Otherwise the PR
        gate in validate_worker.py would force the author to bump every dep's
        Cargo.toml — version bumps belong to bundle release, not PR CI."""
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "lib.rs").write_text("// edit\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["source_changed"] == ["harness"]
        assert "dep-rust" not in data["source_changed"]
        assert "dep-node" not in data["source_changed"]

    def test_harness_metadata_change_does_not_fan_out(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "README.md").write_text("# harness\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness docs"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["harness"]
        assert data["source_changed"] == []

    def test_fanned_out_deps_bucketed_by_language(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "lib.rs").write_text("// edit\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert sorted(data["by_language"]["rust"]) == ["dep-rust", "harness"]
        assert data["by_language"]["node"] == ["dep-node"]
        assert data["by_language"]["python"] == []
