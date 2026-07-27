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


def run_script(
    repo: Path,
    base: str,
    head: str = "HEAD",
    *extra_args: str,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--base",
            base,
            "--head",
            head,
            *extra_args,
        ],
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
        assert data["integration_changed"] is False
        assert data["e2e_changed"] is False


def make_repo_with_harness(tmp_path: Path) -> Path:
    """Tmp repo with harness, its deps, Console, and one unrelated worker."""
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
    (tmp_path / "console").mkdir()
    (tmp_path / "console" / "iii.worker.yaml").write_text(
        'iii: v1\nname: console\nlanguage: node\ndeploy: image\nmanifest: package.json\n'
    )
    (tmp_path / "console" / "package.json").write_text(
        '{"name":"console","version":"0.1.0"}'
    )
    (tmp_path / "provider-anthropic").mkdir()
    (tmp_path / "provider-anthropic" / "iii.worker.yaml").write_text(
        'iii: v1\nname: provider-anthropic\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
    )
    (tmp_path / "provider-anthropic" / "Cargo.toml").write_text(
        '[package]\nname = "provider-anthropic"\nversion = "0.1.0"\n'
    )
    (tmp_path / ".github" / "workflows").mkdir(parents=True)
    (tmp_path / ".github" / "workflows" / "ci.yml").write_text(
        "name: fixture-ci\n"
    )
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


class TestHarnessSelection:
    def test_harness_source_change_selects_only_harness(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "lib.rs").write_text("// breaking change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["harness"]
        assert data["integration_changed"] is True
        assert data["e2e_changed"] is True

    def test_harness_source_change_is_the_only_source_change(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "lib.rs").write_text("// edit\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["harness"]
        assert data["source_changed"] == ["harness"]

    def test_harness_metadata_change_stays_direct(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "README.md").write_text("# harness\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness docs"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["harness"]
        assert data["source_changed"] == []
        assert data["integration_changed"] is False
        assert data["e2e_changed"] is False

    def test_harness_change_is_the_only_rust_worker(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "harness" / "lib.rs").write_text("// edit\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "harness edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["by_language"]["rust"] == ["harness"]
        assert data["by_language"]["node"] == []
        assert data["by_language"]["python"] == []

    def test_harness_manifest_change_runs_integration_without_source_change(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        manifest = repo / "harness" / "Cargo.toml"
        manifest.write_text(manifest.read_text() + "\n[features]\ndefault = []\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "manifest"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["source_changed"] == []
        assert data["integration_changed"] is True
        assert data["e2e_changed"] is True

    def test_unrelated_worker_change_skips_integration(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "unrelated" / "worker.py").write_text("# change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "unrelated"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["integration_changed"] is False
        assert data["e2e_changed"] is False

    def test_force_harness_selects_only_harness_and_runs_integration(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        r = run_script(repo, "HEAD", "HEAD", "--force-worker", "harness")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["harness"]
        assert data["source_changed"] == []
        assert data["integration_changed"] is True
        assert data["e2e_changed"] is True

    def test_console_change_runs_integration(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "console" / "ui.ts").write_text("// change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "console"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["console"]
        assert data["integration_changed"] is True
        assert data["e2e_changed"] is False

    def test_provider_change_runs_only_e2e(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        (repo / "provider-anthropic" / "lib.rs").write_text("// change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "provider"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == ["provider-anthropic"]
        assert data["integration_changed"] is False
        assert data["e2e_changed"] is True

    def test_e2e_suite_change_does_not_run_integration(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        path = repo / "harness" / "tests" / "e2e" / "scenario.rs"
        path.parent.mkdir(parents=True)
        path.write_text("// change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "e2e"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["integration_changed"] is False
        assert data["e2e_changed"] is True

    def test_integration_suite_change_does_not_run_e2e(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        path = repo / "harness" / "tests" / "integration" / "scenario.rs"
        path.parent.mkdir(parents=True)
        path.write_text("// change\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "integration"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["integration_changed"] is True
        assert data["e2e_changed"] is False

    def test_integration_workflow_change_runs_integration(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        workflow = repo / ".github" / "workflows" / "ci.yml"
        workflow.write_text("name: changed-ci\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "ci"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == []
        assert data["integration_changed"] is True
        assert data["e2e_changed"] is True

    def test_e2e_nightly_workflow_change_runs_only_e2e(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        workflow = repo / ".github" / "workflows" / "harness-e2e-nightly.yml"
        workflow.write_text("name: nightly\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "nightly"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["changed_workers"] == []
        assert data["integration_changed"] is False
        assert data["e2e_changed"] is True

    def test_unknown_forced_worker_is_rejected(self, tmp_path):
        repo = make_repo_with_harness(tmp_path)
        r = run_script(repo, "HEAD", "HEAD", "--force-worker", "missing")
        assert r.returncode == 2
        assert "unknown --force-worker: missing" in r.stderr


def make_repo_with_crate(tmp_path: Path) -> Path:
    """Tmp repo: a shared crate under crates/, one worker path-depending on
    it, and one unrelated rust worker."""
    def run(*args):
        return subprocess.run(args, cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    run("git", "init", "-q", "-b", "main")
    run("git", "config", "user.email", "t@e.com")
    run("git", "config", "user.name", "T")
    (tmp_path / "crates" / "shared" / "src").mkdir(parents=True)
    (tmp_path / "crates" / "shared" / "Cargo.toml").write_text(
        '[package]\nname = "iii-shared"\nversion = "0.1.0"\n'
    )
    (tmp_path / "crates" / "shared" / "src" / "lib.rs").write_text("// lib\n")
    (tmp_path / "dependent").mkdir()
    (tmp_path / "dependent" / "iii.worker.yaml").write_text(
        'iii: v1\nname: dependent\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
    )
    (tmp_path / "dependent" / "Cargo.toml").write_text(
        '[package]\nname = "dependent"\nversion = "0.1.0"\n\n[dependencies]\n'
        'iii-shared = { path = "../crates/shared" }\n'
    )
    (tmp_path / "unrelated").mkdir()
    (tmp_path / "unrelated" / "iii.worker.yaml").write_text(
        'iii: v1\nname: unrelated\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n'
    )
    (tmp_path / "unrelated" / "Cargo.toml").write_text(
        '[package]\nname = "unrelated"\nversion = "0.1.0"\n'
    )
    run("git", "add", ".")
    run("git", "commit", "-q", "-m", "init")
    return tmp_path


class TestCrateFanOut:
    def test_crate_source_change_fans_out_to_path_dependents(self, tmp_path):
        repo = make_repo_with_crate(tmp_path)
        (repo / "crates" / "shared" / "src" / "lib.rs").write_text("// edit\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "crate edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["crates"] == ["shared"]
        assert data["changed_workers"] == ["dependent"]
        assert data["by_language"]["rust"] == ["dependent"]
        assert "unrelated" not in data["changed_workers"]
        # The dependent joins the matrix (like --force-worker picks) but is
        # NOT source_changed (no version-bump gate on the PR author).
        assert data["source_changed"] == []

    def test_crate_manifest_change_fans_out(self, tmp_path):
        """Cargo.toml is metadata for workers but source for crates — a dep
        bump in the crate changes what dependents build against."""
        repo = make_repo_with_crate(tmp_path)
        (repo / "crates" / "shared" / "Cargo.toml").write_text(
            '[package]\nname = "iii-shared"\nversion = "0.1.1"\n'
        )
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "crate manifest"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["crates"] == ["shared"]
        assert data["changed_workers"] == ["dependent"]

    def test_crate_docs_only_change_does_not_fan_out(self, tmp_path):
        repo = make_repo_with_crate(tmp_path)
        (repo / "crates" / "shared" / "README.md").write_text("# shared\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "crate docs"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        assert data["crates"] == []
        assert data["changed_workers"] == []

    def test_crate_dir_is_not_a_worker(self, tmp_path):
        repo = make_repo_with_crate(tmp_path)
        (repo / "crates" / "shared" / "src" / "lib.rs").write_text("// edit\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "crate edit"], cwd=repo, check=True, env=GIT_HERMETIC_ENV)
        r = run_script(repo, "main~1")
        assert r.returncode == 0, r.stderr
        data = json.loads(r.stdout)
        # The crate never appears as a changed *worker* — it has no
        # iii.worker.yaml and must not hit worker gates like validate_worker.
        assert "shared" not in data["changed_workers"]
        assert "crates" not in data["changed_workers"]
