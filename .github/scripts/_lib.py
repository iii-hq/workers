"""Shared helpers for .github/scripts/* CLI tools."""
from __future__ import annotations

SemverKey = tuple[tuple[int, ...], int, str]


def parse_semver(v: str) -> SemverKey:
    """Returns a tuple suitable for lexicographic compare.

    Shape: (core_tuple, 1 if stable else 0, pre_suffix).
    The middle int makes stable strictly greater than any pre-release at the
    same core (1.2.3 > 1.2.3-rc.1). The trailing string lexically orders
    multiple pre-releases at the same core (rc.1 < rc.2).
    """
    # Strip build metadata (semver 2.0.0 §10: ignored for precedence).
    v_nobuild, _, _ = v.partition("+")
    core, _, pre = v_nobuild.partition("-")
    parts = [int(x) for x in core.split(".")]
    while len(parts) < 3:
        parts.append(0)
    return (tuple(parts), 0 if pre else 1, pre)
