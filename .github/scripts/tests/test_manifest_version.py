"""Tests for .github/scripts/manifest_version.py."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

import _lib

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

    def test_applies_unnumbered_prerelease_suffix(self, cargo_manifest):
        r = run_script("bump", str(cargo_manifest), "--kind", "minor", "--suffix", "alpha")
        assert r.returncode == 0, r.stderr
        assert r.stdout.strip() == "0.2.0-alpha"

    def test_exact_target_is_authoritative(self, cargo_manifest):
        r = run_script(
            "bump",
            str(cargo_manifest),
            "--kind",
            "major",
            "--suffix",
            "beta",
            "--target",
            "0.3.0-alpha",
        )
        assert r.returncode == 0, r.stderr
        assert r.stdout.strip() == "0.3.0-alpha"

    def test_allows_forward_maturity_jump(self, cargo_manifest):
        cargo_manifest.write_text(
            '[package]\nname = "smoke"\nversion = "0.1.0-experimental"\n'
        )
        r = run_script("bump", str(cargo_manifest), "--kind", "none", "--suffix", "beta")
        assert r.returncode == 0, r.stderr
        assert r.stdout.strip() == "0.1.0-beta"

    @pytest.mark.parametrize("target", ["0.0.9", "0.1.0-preview", "0.1.0-alpha.1"])
    def test_rejects_backwards_or_unsupported_exact_target(self, cargo_manifest, target):
        r = run_script("bump", str(cargo_manifest), "--kind", "none", "--target", target)
        assert r.returncode != 0

    def test_allows_manifest_version_as_is(self, cargo_manifest):
        r = run_script("bump", str(cargo_manifest), "--kind", "none", "--target", "0.1.0")
        assert r.returncode == 0, r.stderr

    def test_allows_first_prerelease_from_unreleased_stable_manifest(self, cargo_manifest):
        r = run_script("bump", str(cargo_manifest), "--kind", "none", "--suffix", "alpha")
        assert r.returncode == 0, r.stderr
        assert r.stdout.strip() == "0.1.0-alpha"


class TestMaturitySubcommand:
    @pytest.mark.parametrize(
        ("version", "expected"),
        [
            ("1.2.3-experimental", "experimental"),
            ("1.2.3-alpha", "alpha"),
            ("1.2.3-beta", "beta"),
            ("1.2.3", "stable"),
        ],
    )
    def test_classifies_supported_versions(self, version, expected):
        r = run_script("maturity", version)
        assert r.returncode == 0
        assert r.stdout.strip() == expected


class TestReleaseHistory:
    def test_allows_forward_maturity_and_skips(self):
        _lib.validate_release_history("1.2.3-beta", ["1.2.3-experimental"])

    @pytest.mark.parametrize("target", ["1.2.3-experimental", "1.2.3-alpha"])
    def test_rejects_maturity_regression(self, target):
        with pytest.raises(ValueError, match="cannot follow"):
            _lib.validate_release_history(target, ["1.2.3-beta"])

    def test_rejects_older_core(self):
        with pytest.raises(ValueError, match="behind existing"):
            _lib.validate_release_history("1.2.3", ["1.3.0-experimental"])

    def test_allows_idempotent_existing_target_for_workflow_check(self):
        _lib.validate_release_history("1.2.3-alpha", ["1.2.3-alpha"])


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


class TestSyncManifests:
    """`sync-tags`: manifests and every lock hold the version the next release publishes.

    Release Control publishes one patch past the highest published version and
    builds the binary from the manifest before it publishes, so the manifest
    must already hold that next version for the binary to report it.
    """

    @pytest.mark.parametrize(
        ("published", "expected"),
        [
            ("0.7.2", "0.7.3"),
            ("1.9.0", "1.9.1"),
            # The successor of a prerelease is not a patch bump: keep it as is.
            ("0.4.0-rc.2", "0.4.0-rc.2"),
            ("0.2.0-experimental", "0.2.0-experimental"),
        ],
    )
    def test_next_release_version(self, published, expected):
        import manifest_version

        assert manifest_version.next_release_version(published) == expected

    def test_writes_next_release_version_into_manifests_and_every_lock(self, tmp_path):
        from types import SimpleNamespace

        import manifest_version

        def write(relative: str, body: str) -> Path:
            path = tmp_path / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(body)
            return path

        write("harness/Cargo.toml", '[package]\nname = "harness"\nversion = "1.8.8-rc.3"\n')
        write("harness/Cargo.lock", 'version = 4\n\n[[package]]\nname = "harness"\nversion = "1.8.8-rc.3"\n')
        write("eval/Cargo.toml", '[package]\nname = "eval"\nversion = "0.2.2"\n')
        # eval depends on harness by path, so its lock pins harness as well.
        eval_lock = write(
            "eval/Cargo.lock",
            'version = 4\n\n[[package]]\nname = "eval"\nversion = "0.2.2"\n\n'
            '[[package]]\nname = "harness"\nversion = "1.8.8-rc.3"\n',
        )
        write("hermes/pyproject.toml", '[project]\nname = "hermes"\nversion = "0.1.7-rc.4"\n')
        write("hermes/uv.lock", 'version = 1\n\n[[package]]\nname = "hermes"\nversion = "0.1.7rc4"\nsource = { editable = "." }\n')
        write("pi/package.json", '{\n  "name": "pi",\n  "version": "0.1.13"\n}\n')
        write("seed/Cargo.toml", '[package]\nname = "seed"\nversion = "0.3.0"\n')
        # Manifest already at the next version, lock left behind by a hand bump.
        write("cron/Cargo.toml", '[package]\nname = "cron"\nversion = "0.21.26"\n')
        write("rcw/Cargo.toml", '[package]\nname = "rcw"\nversion = "0.3.9"\n')
        cron_lock = write("cron/Cargo.lock", 'version = 4\n\n[[package]]\nname = "cron"\nversion = "0.21.11-rc.1"\n')
        write("snake/pyproject.toml", '[project]\nname = "snake"\nversion = "0.1.0"\n')
        catalog = {
            name: SimpleNamespace(path=tmp_path / name, manifest=manifest)
            for name, manifest in {
                "harness": "Cargo.toml", "eval": "Cargo.toml", "hermes": "pyproject.toml",
                "pi": "package.json", "seed": "Cargo.toml", "snake": "pyproject.toml", "cron": "Cargo.toml",
                "rcw": "Cargo.toml",
            }.items()
        }
        tags = [
            "harness/v1.8.36-rc.2", "harness/v1.8.36", "harness/v1.9.0-dry-run.1",
            "eval/v0.2.14", "hermes/v0.1.9", "pi/v0.1.33", "cron/v0.21.25",
            "seed/v0.2.0",  # a hand-bumped manifest ahead of its tag stays
            "snake/v0.1.5", "snake/v0.2.0-experimental",  # the latter has no PEP 440 spelling
            "rcw/v0.3.9", "rcw/v0.4.0-rc.2",  # a prerelease's successor is not predicted
            "unlisted/v9.9.9",
        ]
        locks = [tmp_path / p for p in ("harness/Cargo.lock", "eval/Cargo.lock", "hermes/uv.lock", "cron/Cargo.lock")]

        changes, errors = manifest_version.sync_manifests(catalog, tags, locks)

        assert changes == [
            "eval 0.2.2 -> 0.2.15",
            "harness 1.8.8-rc.3 -> 1.8.37",
            "hermes 0.1.7-rc.4 -> 0.1.10",
            "pi 0.1.13 -> 0.1.34",
            "rcw 0.3.9 -> 0.4.0-rc.2",
            "snake 0.1.0 -> 0.1.6",
        ]
        assert errors == []
        assert _lib.read_version(tmp_path / "harness/Cargo.toml") == "1.8.37"
        assert 'name = "harness"\nversion = "1.8.37"' in (tmp_path / "harness/Cargo.lock").read_text()
        assert 'name = "eval"\nversion = "0.2.15"' in eval_lock.read_text()
        assert 'name = "harness"\nversion = "1.8.37"' in eval_lock.read_text()
        assert 'name = "hermes"\nversion = "0.1.10"' in (tmp_path / "hermes/uv.lock").read_text()
        assert _lib.read_version(tmp_path / "pi/package.json") == "0.1.34"
        assert _lib.read_version(tmp_path / "seed/Cargo.toml") == "0.3.0"
        assert 'name = "cron"\nversion = "0.21.26"' in cron_lock.read_text()
        assert _lib.read_version(tmp_path / "snake/pyproject.toml") == "0.1.6"
        assert _lib.read_version(tmp_path / "rcw/Cargo.toml") == "0.4.0-rc.2"

        assert manifest_version.sync_manifests(catalog, tags, locks)[0] == []
