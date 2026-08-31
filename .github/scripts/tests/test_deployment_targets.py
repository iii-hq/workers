from __future__ import annotations

import pytest

import deployment_targets


def test_binary_default_is_the_complete_unix_matrix() -> None:
    assert deployment_targets.normalize_targets(None, deploy="binary") == [
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-gnu",
        "armv7-unknown-linux-gnueabihf",
    ]


def test_windows_and_unknown_targets_are_rejected() -> None:
    with pytest.raises(ValueError, match="stable-profile only"):
        deployment_targets.normalize_targets("x86_64-pc-windows-msvc")
    with pytest.raises(ValueError, match="unknown release target"):
        deployment_targets.normalize_targets("sparc64-unknown-linux-gnu")


def test_allow_windows_accepts_the_supported_triple_and_orders_it_last() -> None:
    assert deployment_targets.normalize_targets(
        ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "x86_64-apple-darwin"],
        allow_windows=True,
    ) == ["x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]


def test_unsupported_windows_triples_stay_rejected_even_with_allow_windows() -> None:
    with pytest.raises(ValueError, match="Windows release target is not supported"):
        deployment_targets.normalize_targets("i686-pc-windows-msvc", allow_windows=True)


def test_explicit_subsets_are_canonicalized_and_non_binary_has_one_build() -> None:
    assert deployment_targets.normalize_targets(
        ["aarch64-unknown-linux-gnu", "x86_64-apple-darwin"], deploy="binary"
    ) == ["x86_64-apple-darwin", "aarch64-unknown-linux-gnu"]
