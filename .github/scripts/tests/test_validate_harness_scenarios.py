import json
import pathlib
import subprocess
import sys


SCRIPT = pathlib.Path(__file__).parents[1] / "validate_harness_scenarios.py"
DIGEST = "a" * 64


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], text=True, capture_output=True, check=False)


def test_accepts_the_exact_requested_scenarios_and_policy_identity():
    result = run(
        "--available-json",
        '["one","two"]',
        "--requested-json",
        '["two"]',
        "--required-json",
        '["two"]',
        "--profile",
        "release",
        "--policy-digest",
        DIGEST,
        "--policy-version",
        "3",
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == {
        "profile_digest": DIGEST,
        "policy_version": 3,
        "required_scenarios": ["two"],
        "scenarios": ["two"],
        "validation_profile": "release",
    }


def test_empty_selection_is_rejected():
    result = run(
        "--available-json",
        '["one","two"]',
        "--requested-json",
        "[]",
        "--required-json",
        "[]",
        "--profile",
        "full",
        "--policy-digest",
        DIGEST,
        "--policy-version",
        "3",
    )
    assert result.returncode != 0


def test_rejects_unknown_or_duplicate_scenarios_and_invalid_policy_identity():
    cases = [
        ('["one"]', '["two"]', '["two"]', DIGEST, "3"),
        ('["one"]', '["one","one"]', '["one"]', DIGEST, "3"),
        ('["one"]', '["one"]', '["two"]', DIGEST, "3"),
        ('["one"]', '["one"]', '["one"]', "not-a-digest", "3"),
        ('["one"]', '["one"]', '["one"]', DIGEST, "0"),
    ]
    for available, requested, required, digest, version in cases:
        result = run(
            "--available-json",
            available,
            "--requested-json",
            requested,
            "--required-json",
            required,
            "--profile",
            "custom",
            "--policy-digest",
            digest,
            "--policy-version",
            version,
        )
        assert result.returncode != 0
