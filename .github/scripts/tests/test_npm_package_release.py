"""Tests for .github/scripts/npm_package_release.py."""
from __future__ import annotations

import io
import json
import re
import subprocess
import tarfile
from pathlib import Path

import pytest
import yaml

import npm_package_release as release
from _test_helpers import GIT_HERMETIC_ENV

ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = ROOT / ".github" / "workflows" / "publish-npm-packages.yml"


def tarball(files: dict[str, str | dict]) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for name, content in files.items():
            data = (json.dumps(content) if isinstance(content, dict) else content).encode()
            info = tarfile.TarInfo(f"package/{name}")
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True, env=GIT_HERMETIC_ENV
    ).stdout


class TestBump:
    @pytest.mark.parametrize(
        ("version", "kind", "expected"),
        [
            ("0.4.0", "patch", "0.4.1"),
            ("0.4.3", "minor", "0.5.0"),
            ("0.4.3", "major", "0.5.0"),
            ("1.2.3", "major", "2.0.0"),
            ("1.2.3", "minor", "1.3.0"),
        ],
    )
    def test_bump_version(self, version, kind, expected):
        assert release.bump_version(version, kind) == expected

    @pytest.mark.parametrize("version", ["1.2", "1.2.3-beta", "01.2.3", "v1.2.3"])
    def test_rejects_non_release_versions(self, version):
        with pytest.raises(ValueError):
            release.parse_version(version)

    @pytest.mark.parametrize(
        ("messages", "expected"),
        [
            ([], "patch"),
            (["fix(ade): a thing", "chore: tidy"], "patch"),
            (["fix: a", "feat(ade,workflow): b (#1)"], "minor"),
            (["feat!: drop observe"], "major"),
            (["refactor(console-ui)!: rename"], "major"),
            (["fix: a\n\nBREAKING CHANGE: hooks moved"], "major"),
            (["docs: mention feat: in a body\n\nfeat: not a subject"], "patch"),
        ],
    )
    def test_bump_kind(self, messages, expected):
        assert release.bump_kind(messages) == expected


class TestDecide:
    def plan(self, **overrides):
        args = {
            "current": "0.4.0",
            "latest": "0.4.0",
            "published": {"0.3.0", "0.4.0"},
            "pending": [],
            "same_as_latest": False,
            "messages": ["feat: x"],
            "bump": "auto",
        }
        args.update(overrides)
        return release.decide(**args)

    def test_first_publication_stages_package_json_as_is(self):
        plan = self.plan(current="0.1.0", latest=None, published=set(), same_as_latest=None)
        assert (plan.action, plan.version, plan.sync) == ("stage", "0.1.0", "")

    def test_changed_files_bump_from_the_commits(self):
        assert (self.plan().action, self.plan().version) == ("stage", "0.5.0")
        assert self.plan(messages=["fix: x"]).version == "0.4.1"

    def test_unchanged_files_stage_nothing(self):
        plan = self.plan(same_as_latest=True)
        assert (plan.action, plan.sync) == ("none", "")

    def test_an_approved_stage_is_synced_into_package_json(self):
        plan = self.plan(current="0.4.0", latest="0.5.0", published={"0.4.0", "0.5.0"}, same_as_latest=True)
        assert (plan.action, plan.sync) == ("none", "0.5.0")

    def test_registry_ahead_with_new_changes_syncs_and_bumps_from_the_registry(self):
        plan = self.plan(current="0.4.0", latest="0.5.0", published={"0.4.0", "0.5.0"}, messages=["fix: x"])
        assert (plan.action, plan.version, plan.sync) == ("stage", "0.5.1", "0.5.0")

    def test_a_pending_stage_blocks_the_next_one(self):
        plan = self.plan(pending=["0.5.0"])
        assert (plan.action, plan.version) == ("awaiting", "0.5.0")
        assert self.plan(pending=["0.5.0"], bump="major").action == "awaiting"

    def test_hand_bump_ahead_of_the_registry_is_kept(self):
        plan = self.plan(current="1.0.0", same_as_latest=None)
        assert (plan.action, plan.version) == ("stage", "1.0.0")

    def test_hand_bump_already_published_but_not_latest_fails(self):
        with pytest.raises(ValueError, match="dist-tag"):
            self.plan(current="0.5.0", published={"0.4.0", "0.5.0"}, same_as_latest=None)

    def test_forced_bump_stages_even_unchanged_files(self):
        plan = self.plan(bump="patch", same_as_latest=None)
        assert (plan.action, plan.version) == ("stage", "0.4.1")
        assert self.plan(bump="major", current="1.0.0", latest="0.4.0", same_as_latest=None).version == "2.0.0"

    def test_forced_bump_never_reuses_a_published_version(self):
        with pytest.raises(ValueError, match="already"):
            self.plan(bump="patch", current="0.4.0", latest="0.4.0", published={"0.4.0", "0.4.1"})


class TestPendingStages:
    def test_only_open_stages_of_unpublished_versions_of_this_package(self):
        items = [
            {"id": "a", "packageName": "@x/p", "version": "0.5.0", "status": "pending"},
            {"id": "b", "packageName": "@x/p", "version": "0.4.0", "status": "approved"},
            {"id": "c", "packageName": "@x/p", "version": "0.6.0", "status": "Rejected"},
            {"id": "d", "packageName": "@x/other", "version": "9.0.0", "status": "pending"},
            {"id": "e", "packageName": "@x/p", "version": "0.10.0", "status": "awaiting_approval"},
        ]
        assert release.pending_stages(items, "@x/p", {"0.4.0"}) == ["0.5.0", "0.10.0"]

    def test_nothing_staged(self):
        assert release.pending_stages([], "@x/p", set()) == []


class TestTarballContents:
    def test_ignores_version_dev_dependencies_and_git_head(self):
        published = tarball(
            {
                "package.json": {"name": "x", "version": "0.4.0", "devDependencies": {"react": "catalog:"}},
                "index.js": "a",
            }
        )
        packed = tarball(
            {
                "package.json": {"name": "x", "version": "0.5.0", "devDependencies": {"react": "^19"}, "gitHead": "abc"},
                "index.js": "a",
            }
        )
        assert release.tarball_contents(published) == release.tarball_contents(packed)

    def test_sees_shipped_changes(self):
        base = {"package.json": {"name": "x", "version": "1.0.0", "dependencies": {"a": "^1"}}, "index.js": "a"}
        assert release.tarball_contents(tarball(base)) != release.tarball_contents(
            tarball({**base, "index.js": "b"})
        )
        assert release.tarball_contents(tarball(base)) != release.tarball_contents(
            tarball({**base, "extra.js": "x"})
        )
        assert release.tarball_contents(tarball(base)) != release.tarball_contents(
            tarball({**base, "package.json": {"name": "x", "version": "1.0.0", "dependencies": {"a": "^2"}}})
        )


class TestWriteVersion:
    def test_rewrites_only_the_package_version(self, tmp_path):
        text = '{\n  "name": "x",\n  "version": "0.4.0",\n  "dependencies": {\n    "version": "1.0.0"\n  }\n}\n'
        (tmp_path / "package.json").write_text(text)
        release.write_version(tmp_path, "0.5.0")
        assert (tmp_path / "package.json").read_text() == text.replace('"0.4.0"', '"0.5.0"')

    def test_rejects_a_bad_version(self, tmp_path):
        (tmp_path / "package.json").write_text('{"name": "x", "version": "0.4.0"}')
        with pytest.raises(ValueError):
            release.write_version(tmp_path, "0.5")


class TestHistory:
    def test_commits_since_the_last_version_change_in_the_package_only(self, tmp_path):
        git(tmp_path, "init", "-q", "-b", "main")
        git(tmp_path, "config", "user.email", "t@example.com")
        git(tmp_path, "config", "user.name", "t")
        pkg = tmp_path / "packages" / "p"
        pkg.mkdir(parents=True)

        def commit(path: Path, content: str, message: str) -> None:
            path.write_text(content)
            git(tmp_path, "add", "-A")
            git(tmp_path, "commit", "-q", "-m", message)

        commit(pkg / "package.json", '{\n  "name": "p",\n  "version": "0.1.0"\n}\n', "feat: initial")
        commit(pkg / "index.js", "a", "feat: before the release")
        commit(pkg / "package.json", '{\n  "name": "p",\n  "version": "0.2.0"\n}\n', "chore(release): 0.2.0")
        commit(pkg / "index.js", "b", "fix(p): after the release")
        commit(tmp_path / "other.txt", "x", "feat(other): outside the package")
        commit(pkg / "package.json", '{\n  "name": "p",\n  "version": "0.2.0",\n  "main": "index.js"\n}\n', "fix(p): main")

        baseline = release.release_baseline(pkg)
        assert git(tmp_path, "log", "-1", "--format=%s", baseline).strip() == "chore(release): 0.2.0"
        assert release.commit_messages_since(pkg, baseline) == ["fix(p): main", "fix(p): after the release"]

    def test_no_version_change_means_the_whole_history(self, tmp_path):
        git(tmp_path, "init", "-q", "-b", "main")
        git(tmp_path, "config", "user.email", "t@example.com")
        git(tmp_path, "config", "user.name", "t")
        (tmp_path / "index.js").write_text("a")
        git(tmp_path, "add", "-A")
        git(tmp_path, "commit", "-q", "-m", "feat: a")
        assert release.release_baseline(tmp_path) is None
        assert release.commit_messages_since(tmp_path, None) == ["feat: a"]


class TestWorkflow:
    def test_bot_commit_uses_the_subject_release_control_skips_and_is_indexed(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        assert "--message 'chore(release): sync package manifests to published versions'" in text
        assert "gh workflow run deploy-descriptor-index.yml --ref main" in text

    def test_the_token_only_stages_and_an_hourly_run_syncs_approvals(self):
        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        assert workflow[True]["schedule"]
        commands = [step.get("run", "") for step in workflow["jobs"]["publish"]["steps"]]
        assert any("npm stage publish" in command for command in commands)
        assert not any(re.search(r"\bnpm publish\b", command) for command in commands)

    def test_dispatch_values_are_never_rendered_into_shell_source(self):
        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        for job in workflow["jobs"].values():
            for step in job.get("steps", []):
                command = step.get("run", "")
                assert "${{" not in command, step.get("name", "unnamed")

    def test_published_packages_name_this_repository_for_provenance(self):
        for package in ("console-ui", "native-ui"):
            manifest = json.loads((ROOT / "packages" / package / "package.json").read_text())
            assert manifest["repository"]["url"] == "git+https://github.com/iii-hq/workers.git"
            assert manifest["repository"]["directory"] == f"packages/{package}"
            assert not manifest.get("private")
