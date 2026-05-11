"""Tests for .github/scripts/_lib.py."""
from __future__ import annotations

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
