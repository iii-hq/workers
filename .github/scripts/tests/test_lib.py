"""Tests for .github/scripts/_lib.py."""
from __future__ import annotations

from pathlib import Path

import pytest

import _lib


class TestParseSemver:
    def test_stable_three_part(self):
        assert _lib.parse_semver("1.2.3") == ((1, 2, 3), 1, "")

    def test_stable_pads_to_three(self):
        assert _lib.parse_semver("1.2") == ((1, 2, 0), 1, "")

    def test_prerelease_strips_core_keeps_suffix(self):
        assert _lib.parse_semver("1.2.3-rc.1") == ((1, 2, 3), 0, "rc.1")

    def test_stable_greater_than_prerelease_same_core(self):
        # 1.2.3 must rank above 1.2.3-rc.1 (the audit bug).
        assert _lib.parse_semver("1.2.3") > _lib.parse_semver("1.2.3-rc.1")

    def test_prerelease_greater_core_beats_stable(self):
        # 1.2.4-rc.1 must rank above 1.2.3 (newer core wins).
        assert _lib.parse_semver("1.2.4-rc.1") > _lib.parse_semver("1.2.3")

    def test_two_prereleases_sort_lexicographically(self):
        assert _lib.parse_semver("1.2.3-rc.1") < _lib.parse_semver("1.2.3-rc.2")

    def test_build_metadata_is_ignored(self):
        # SemVer 2.0.0 §10: build metadata after `+` MUST NOT affect precedence.
        assert _lib.parse_semver("1.0.0+build.5") == ((1, 0, 0), 1, "")
        assert _lib.parse_semver("1.0.0+a") == _lib.parse_semver("1.0.0+b")


class TestBump:
    def test_patch(self):
        assert _lib.bump("1.2.3", "patch") == "1.2.4"

    def test_minor_resets_patch(self):
        assert _lib.bump("1.2.3", "minor") == "1.3.0"

    def test_major_resets_minor_and_patch(self):
        assert _lib.bump("1.2.3", "major") == "2.0.0"

    def test_strips_prerelease_suffix(self):
        # `bump("1.2.3-rc.1", "patch")` should yield "1.2.4" — bumping a
        # pre-release means moving past the stable release at the same core.
        assert _lib.bump("1.2.3-rc.1", "patch") == "1.2.4"

    def test_pads_short_version(self):
        assert _lib.bump("1", "patch") == "1.0.1"


class TestDetectKind:
    def test_cargo(self, tmp_path):
        assert _lib.detect_kind(tmp_path / "Cargo.toml") == "cargo"

    def test_node(self, tmp_path):
        assert _lib.detect_kind(tmp_path / "package.json") == "node"

    def test_python(self, tmp_path):
        assert _lib.detect_kind(tmp_path / "pyproject.toml") == "python"

    def test_unknown_raises(self, tmp_path):
        with pytest.raises(ValueError):
            _lib.detect_kind(tmp_path / "Makefile")


class TestReadVersionCargo:
    def test_reads_first_package_version(self, cargo_manifest):
        assert _lib.read_version(cargo_manifest) == "0.1.0"

    def test_ignores_dependency_versions(self, tmp_path):
        p = tmp_path / "Cargo.toml"
        p.write_text(
            '[package]\nname = "x"\nversion = "1.0.0"\n'
            '[dependencies]\nfoo = { version = "2.0.0" }\n'
        )
        assert _lib.read_version(p) == "1.0.0"

    def test_missing_version_raises(self, tmp_path):
        p = tmp_path / "Cargo.toml"
        p.write_text('[package]\nname = "x"\n')
        with pytest.raises(ValueError):
            _lib.read_version(p)


class TestWriteVersionCargo:
    def test_writes_and_round_trips(self, cargo_manifest):
        _lib.write_version(cargo_manifest, "9.9.9")
        assert _lib.read_version(cargo_manifest) == "9.9.9"

    def test_preserves_other_lines(self, cargo_manifest):
        _lib.write_version(cargo_manifest, "9.9.9")
        text = cargo_manifest.read_text()
        assert 'name = "smoke"' in text
        assert 'edition = "2021"' in text


class TestReadVersionNode:
    def test_reads_version(self, package_json_manifest):
        assert _lib.read_version(package_json_manifest) == "0.1.0"

    def test_missing_version_raises(self, tmp_path):
        p = tmp_path / "package.json"
        p.write_text('{"name": "x"}')
        with pytest.raises(ValueError):
            _lib.read_version(p)


class TestWriteVersionNode:
    def test_writes_and_round_trips(self, package_json_manifest):
        _lib.write_version(package_json_manifest, "9.9.9")
        assert _lib.read_version(package_json_manifest) == "9.9.9"

    def test_preserves_other_keys(self, tmp_path):
        import json
        p = tmp_path / "package.json"
        p.write_text('{\n  "name": "smoke",\n  "version": "0.1.0",\n  "private": true\n}\n')
        _lib.write_version(p, "9.9.9")
        data = json.loads(p.read_text())
        assert data == {"name": "smoke", "version": "9.9.9", "private": True}


class TestReadVersionPython:
    def test_reads_project_version(self, pyproject_manifest):
        assert _lib.read_version(pyproject_manifest) == "0.1.0"

    def test_ignores_non_project_sections(self, tmp_path):
        p = tmp_path / "pyproject.toml"
        p.write_text(
            '[build-system]\nrequires = ["hatchling"]\n'
            '[project]\nname = "x"\nversion = "1.0.0"\n'
            '[tool.foo]\nversion = "9.9.9"\n'
        )
        assert _lib.read_version(p) == "1.0.0"

    def test_missing_version_raises(self, tmp_path):
        p = tmp_path / "pyproject.toml"
        p.write_text('[project]\nname = "x"\n')
        with pytest.raises(ValueError):
            _lib.read_version(p)


class TestWriteVersionPython:
    def test_writes_and_round_trips(self, pyproject_manifest):
        _lib.write_version(pyproject_manifest, "9.9.9")
        assert _lib.read_version(pyproject_manifest) == "9.9.9"

    def test_preserves_trailing_newline(self, tmp_path):
        p = tmp_path / "pyproject.toml"
        p.write_text('[project]\nname = "x"\nversion = "1.0.0"\n')
        _lib.write_version(p, "9.9.9")
        assert p.read_text().endswith("\n")

    def test_only_replaces_project_version(self, tmp_path):
        p = tmp_path / "pyproject.toml"
        p.write_text(
            '[project]\nname = "x"\nversion = "1.0.0"\n'
            '[tool.foo]\nversion = "0.0.1"\n'
        )
        _lib.write_version(p, "9.9.9")
        text = p.read_text()
        assert 'version = "9.9.9"' in text
        assert 'version = "0.0.1"' in text  # untouched


class TestReadIIIWorkerYaml:
    def test_well_formed_rust_binary(self, iii_worker_yaml_dir):
        m = _lib.read_iii_worker_yaml(iii_worker_yaml_dir)
        assert m.name == "smoke"
        assert m.language == "rust"
        assert m.deploy == "binary"
        assert m.manifest == "Cargo.toml"
        assert m.bin == "smoke-bin"
        assert isinstance(m.raw, dict)

    def test_missing_file_raises(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            _lib.read_iii_worker_yaml(tmp_path)

    def test_falls_back_name_to_folder_name(self, tmp_path):
        (tmp_path / "iii.worker.yaml").write_text(
            'iii: v1\nlanguage: node\ndeploy: image\nmanifest: package.json\n'
        )
        m = _lib.read_iii_worker_yaml(tmp_path)
        assert m.name == tmp_path.name
        assert m.language == "node"
        assert m.bin is None

    def test_is_frozen(self, iii_worker_yaml_dir):
        m = _lib.read_iii_worker_yaml(iii_worker_yaml_dir)
        with pytest.raises(Exception):  # FrozenInstanceError
            m.name = "other"  # type: ignore[misc]


class TestReadTagAnnotation:
    def test_parses_key_value_lines(self, tmp_git_repo_with_tag, monkeypatch):
        monkeypatch.chdir(tmp_git_repo_with_tag)
        ann = _lib.read_tag_annotation("smoke/v0.1.0")
        assert ann["registry-tag"] == "next"
        assert ann["other"] == "value"

    def test_missing_tag_returns_empty_dict(self, tmp_git_repo_with_tag, monkeypatch):
        monkeypatch.chdir(tmp_git_repo_with_tag)
        ann = _lib.read_tag_annotation("does/not/exist")
        assert ann == {}

    def test_skips_comments_and_blank_lines(self, tmp_path, monkeypatch):
        import subprocess
        from _test_helpers import GIT_HERMETIC_ENV
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "config", "user.email", "t@e.com"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "config", "user.name", "T"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
        (tmp_path / "x").write_text("x")
        subprocess.run(["git", "add", "."], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(["git", "commit", "-q", "-m", "init"], cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
        subprocess.run(
            ["git", "tag", "-a", "v0", "-m", "# comment\nkey: val\n\nother: more\n"],
            cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV,
        )
        monkeypatch.chdir(tmp_path)
        ann = _lib.read_tag_annotation("v0")
        assert ann == {"key": "val", "other": "more"}


class TestCargoLockSelfVersion:
    LOCK = (
        'version = 3\n\n'
        '[[package]]\n'
        'name = "dep"\n'
        'version = "2.0.0"\n\n'
        '[[package]]\n'
        'name = "harness"\n'
        'version = "1.0.4"\n'
        'dependencies = [\n'
        ' "dep",\n'
        ']\n'
    )

    def test_read_cargo_package_name(self, tmp_path):
        m = tmp_path / "Cargo.toml"
        m.write_text('[package]\nname = "harness"\nversion = "1.0.6"\n')
        assert _lib.read_cargo_package_name(m) == "harness"

    def test_read_name_ignores_dependency_sections(self, tmp_path):
        m = tmp_path / "Cargo.toml"
        m.write_text(
            '[package]\nname = "harness"\nversion = "1.0.6"\n\n'
            '[dependencies]\nname = "not-this"\n'
        )
        assert _lib.read_cargo_package_name(m) == "harness"

    def test_sync_updates_only_self_entry(self, tmp_path):
        lock = tmp_path / "Cargo.lock"
        lock.write_text(self.LOCK)
        changed = _lib.sync_cargo_lock_self_version(lock, "harness", "1.0.6")
        assert changed is True
        body = lock.read_text()
        assert 'name = "harness"\nversion = "1.0.6"' in body
        assert 'name = "dep"\nversion = "2.0.0"' in body  # untouched

    def test_sync_is_idempotent(self, tmp_path):
        lock = tmp_path / "Cargo.lock"
        lock.write_text(self.LOCK.replace('"1.0.4"', '"1.0.6"'))
        before = lock.read_text()
        assert _lib.sync_cargo_lock_self_version(lock, "harness", "1.0.6") is False
        assert lock.read_text() == before

    def test_sync_raises_when_package_absent(self, tmp_path):
        lock = tmp_path / "Cargo.lock"
        lock.write_text(self.LOCK)
        with pytest.raises(ValueError, match="not found"):
            _lib.sync_cargo_lock_self_version(lock, "ghost", "9.9.9")
