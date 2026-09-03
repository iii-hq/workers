from __future__ import annotations

import pytest

import deployment_targets


def test_binary_default_is_the_complete_release_matrix() -> None:
    assert deployment_targets.normalize_targets(None, deploy="binary") == [
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-gnu",
        "armv7-unknown-linux-gnueabihf",
        "x86_64-pc-windows-msvc",
        "i686-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    ]


def test_unknown_targets_are_rejected() -> None:
    with pytest.raises(ValueError, match="unknown release target"):
        deployment_targets.normalize_targets("sparc64-unknown-linux-gnu")
    with pytest.raises(ValueError, match="unknown release target"):
        deployment_targets.normalize_targets("x86_64-pc-windows-gnu")


def test_every_msvc_triple_is_accepted_and_ordered_last() -> None:
    assert deployment_targets.normalize_targets([
        "aarch64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
        "i686-pc-windows-msvc",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]) == [
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        "i686-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    ]


def test_explicit_subsets_are_canonicalized_and_non_binary_has_one_build() -> None:
    assert deployment_targets.normalize_targets(
        ["aarch64-unknown-linux-gnu", "x86_64-apple-darwin"], deploy="binary"
    ) == ["x86_64-apple-darwin", "aarch64-unknown-linux-gnu"]
