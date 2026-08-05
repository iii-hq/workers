"""Tests for .github/scripts/parse_release_tag.py."""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

from _test_helpers import GIT_HERMETIC_ENV

SCRIPT = Path(__file__).resolve().parents[1] / "parse_release_tag.py"


def make_repo_with_tagged_worker(tmp_path, tag, version, deploy="binary",
                                  registry_tag_line="registry-tag: latest"):
    """Init a tmp repo with one worker + an annotated tag matching `tag`."""
    def run(*args):
        return subprocess.run(args, cwd=tmp_path, check=True, env=GIT_HERMETIC_ENV)
    run("git", "init", "-q", "-b", "main")
    run("git", "config", "user.email", "t@e.com")
    run("git", "config", "user.name", "T")
    worker = tag.split("/")[0]
    w = tmp_path / worker
    w.mkdir()
    (w / "iii.worker.yaml").write_text(
        f'iii: v1\nname: {worker}\nlanguage: rust\ndeploy: {deploy}\n'
        f'manifest: Cargo.toml\nbin: {worker}-bin\n'
    )
    (w / "Cargo.toml").write_text(f'[package]\nname = "{worker}"\nversion = "{version}"\n')
    catalog_dir = tmp_path / ".github"
    catalog_dir.mkdir()
    (catalog_dir / "release-workers.yaml").write_text(
        yaml.safe_dump(
            {
                "schema_version": 1,
                "defaults": {
                    "release_workflow": "release.yml",
                    "allow_direct_latest": True,
                    "required_validation": "smoke",
                },
                "standard_workers": [worker],
                "special_workers": {},
                "policies": {},
            }
        )
    )
    run("git", "add", ".")
    run("git", "commit", "-q", "-m", "init")
    body = f"Release {tag}\n\n{registry_tag_line}\n"
    run("git", "tag", "-a", tag, "-m", body)
    return tmp_path


def run_script(repo, raw_tag, github_output_path):
    env = {**GIT_HERMETIC_ENV, "GITHUB_OUTPUT": str(github_output_path)}
    return subprocess.run(
        [sys.executable, str(SCRIPT), raw_tag],
        capture_output=True, text=True, cwd=repo, env=env,
    )


def parse_outputs(path):
    out = {}
    for line in path.read_text().splitlines():
        if "=" in line:
            k, _, v = line.partition("=")
            out[k] = v
    return out


class TestParseReleaseTag:
    def test_stable_binary_tag(self, tmp_path):
        repo = make_repo_with_tagged_worker(tmp_path, "smoke/v1.2.3", "1.2.3")
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "smoke/v1.2.3", out_path)
        assert r.returncode == 0, r.stderr
        out = parse_outputs(out_path)
        assert out["tag"] == "smoke/v1.2.3"
        assert out["worker"] == "smoke"
        assert out["version"] == "1.2.3"
        assert out["deploy"] == "binary"
        assert out["bin"] == "smoke-bin"
        assert out["registry_tag"] == "latest"
        assert out["is_prerelease"] == "false"
        assert out["dry_run"] == "false"
        assert len(out["tag_sha"]) == 40

    def test_prerelease_sets_is_prerelease(self, tmp_path):
        repo = make_repo_with_tagged_worker(tmp_path, "smoke/v1.2.3-rc.1", "1.2.3-rc.1",
                                             registry_tag_line="registry-tag: next")
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "smoke/v1.2.3-rc.1", out_path)
        assert r.returncode == 0
        out = parse_outputs(out_path)
        assert out["is_prerelease"] == "true"
        assert out["dry_run"] == "false"
        assert out["registry_tag"] == "next"

    def test_non_numeric_prerelease_suffix_is_not_promotable_stable(self, tmp_path):
        repo = make_repo_with_tagged_worker(
            tmp_path,
            "smoke/v1.2.3-preview",
            "1.2.3-preview",
            registry_tag_line="registry-tag: next",
        )
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "smoke/v1.2.3-preview", out_path)
        assert r.returncode == 0
        assert parse_outputs(out_path)["is_prerelease"] == "true"

    def test_v2_tag_exposes_release_control_identity(self, tmp_path):
        metadata = "\n".join(
            [
                "release-contract: 2",
                "worker: smoke",
                "version: 1.2.3-alpha",
                "maturity: alpha",
                "registry-tag: next",
                "experimental: false",
                "operation-id: 11111111-1111-1111-1111-111111111111",
                "step-id: 22222222-2222-2222-2222-222222222222",
                f"source-sha: {'a' * 40}",
            ]
        )
        repo = make_repo_with_tagged_worker(
            tmp_path,
            "smoke/v1.2.3-alpha",
            "1.2.3-alpha",
            registry_tag_line=metadata,
        )
        out_path = tmp_path / "gh_output"
        out_path.touch()
        result = run_script(repo, "smoke/v1.2.3-alpha", out_path)
        assert result.returncode == 0, result.stderr
        output = parse_outputs(out_path)
        assert output["release_contract"] == "2"
        assert output["maturity"] == "alpha"
        assert output["operation_id"] == "11111111-1111-1111-1111-111111111111"

    def test_v2_prerelease_cannot_publish_latest(self, tmp_path):
        metadata = "\n".join(
            [
                "release-contract: 2",
                "worker: smoke",
                "version: 1.2.3-beta",
                "maturity: beta",
                "registry-tag: latest",
                "experimental: false",
                "operation-id: op",
                "step-id: step",
                "source-sha: unknown",
            ]
        )
        repo = make_repo_with_tagged_worker(
            tmp_path,
            "smoke/v1.2.3-beta",
            "1.2.3-beta",
            registry_tag_line=metadata,
        )
        out_path = tmp_path / "gh_output"
        out_path.touch()
        result = run_script(repo, "smoke/v1.2.3-beta", out_path)
        assert result.returncode != 0
        assert "must publish to next" in result.stderr

    def test_missing_channel_fails_closed_to_next(self, tmp_path):
        repo = make_repo_with_tagged_worker(
            tmp_path,
            "smoke/v1.2.3",
            "1.2.3",
            registry_tag_line="experimental: false",
        )
        out_path = tmp_path / "gh_output"
        out_path.touch()
        result = run_script(repo, "smoke/v1.2.3", out_path)
        assert result.returncode == 0, result.stderr
        assert parse_outputs(out_path)["registry_tag"] == "next"

    def test_dry_run_tag(self, tmp_path):
        repo = make_repo_with_tagged_worker(tmp_path, "smoke/v9.9.9-dry-run.1", "9.9.9-dry-run.1")
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "smoke/v9.9.9-dry-run.1", out_path)
        assert r.returncode == 0
        out = parse_outputs(out_path)
        assert out["dry_run"] == "true"
        assert out["is_prerelease"] == "true"

    def test_image_deploy(self, tmp_path):
        repo = make_repo_with_tagged_worker(tmp_path, "smoke/v1.0.0", "1.0.0", deploy="image")
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "smoke/v1.0.0", out_path)
        assert r.returncode == 0
        out = parse_outputs(out_path)
        assert out["deploy"] == "image"

    def test_malformed_tag_fails(self, tmp_path):
        repo = make_repo_with_tagged_worker(tmp_path, "smoke/v1.0.0", "1.0.0")
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "not-a-tag", out_path)
        assert r.returncode != 0

    def test_missing_iii_worker_yaml_fails(self, tmp_path):
        repo = make_repo_with_tagged_worker(tmp_path, "smoke/v1.0.0", "1.0.0")
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "missing/v1.0.0", out_path)
        assert r.returncode != 0


class TestExperimentalAnnotation:
    """`experimental:` rides the annotated tag message next to `registry-tag:`."""

    def _run(self, tmp_path, registry_tag_line):
        repo = make_repo_with_tagged_worker(
            tmp_path, "smoke/v1.0.0", "1.0.0", registry_tag_line=registry_tag_line
        )
        out_path = tmp_path / "gh_output"
        out_path.touch()
        r = run_script(repo, "smoke/v1.0.0", out_path)
        assert r.returncode == 0, r.stderr
        return parse_outputs(out_path)

    def test_absent_line_is_false(self, tmp_path):
        out = self._run(tmp_path, "registry-tag: latest")
        assert out["experimental"] == "false"

    def test_true_marks_experimental(self, tmp_path):
        out = self._run(tmp_path, "registry-tag: latest\nexperimental: true")
        assert out["experimental"] == "true"
        assert out["registry_tag"] == "latest"

    def test_false_stays_false(self, tmp_path):
        out = self._run(tmp_path, "registry-tag: latest\nexperimental: false")
        assert out["experimental"] == "false"

    def test_casing_and_padding_tolerated(self, tmp_path):
        out = self._run(tmp_path, "registry-tag: latest\nexperimental:   TRUE  ")
        assert out["experimental"] == "true"

    # A typo must not mark a stable worker experimental — anything but the
    # exact word is false, and the release still publishes.
    def test_typo_falls_back_to_false(self, tmp_path):
        out = self._run(tmp_path, "registry-tag: latest\nexperimental: yes")
        assert out["experimental"] == "false"
