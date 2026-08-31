import pytest

from deployment_effects import classify_effect, mutation_plan


MUTATING_PHASES = ("publish",)


@pytest.mark.parametrize("phase", MUTATING_PHASES)
@pytest.mark.parametrize(
    ("before", "mutation", "after", "expected"),
    [
        ("absent", "not_started", "unknown", "absent"),
        ("absent", "started", "unknown", "unknown"),
        ("absent", "completed", "unknown", "unknown"),
        ("absent", "started", "absent", "absent"),
        ("absent", "started", "present", "present"),
        ("present", "not_started", "unknown", "present"),
        ("present", "not_started", "present", "present"),
        ("unknown", "not_started", "unknown", "unknown"),
    ],
)
def test_crash_before_and_after_each_effect_has_evidence_bound_state(
    phase: str, before: str, mutation: str, after: str, expected: str
) -> None:
    # The phase parameter makes the same invariant explicit for every external
    # effect owner without executing GitHub, Registry, or GHCR mutations.
    assert phase in MUTATING_PHASES
    assert classify_effect(before=before, mutation=mutation, after=after) == expected


@pytest.mark.parametrize(
    ("before", "expected"),
    [("absent", "mutate"), ("present", "skip"), ("unknown", "refuse")],
)
def test_retry_never_blindly_duplicates_an_effect(before: str, expected: str) -> None:
    assert mutation_plan(before) == expected


def test_invalid_states_fail_closed() -> None:
    for invalid in ("", "failed", "maybe"):
        with pytest.raises(ValueError):
            classify_effect(before=invalid, mutation="not_started", after="unknown")
        with pytest.raises(ValueError):
            mutation_plan(invalid)

    for invalid in ("", "crashed", "mutated"):
        with pytest.raises(ValueError):
            classify_effect(before="absent", mutation=invalid, after="unknown")
