import json
import pathlib
import subprocess
import sys


SCRIPT = pathlib.Path(__file__).parents[1] / "release_control_contract.py"
OPERATION_ID = "11111111-1111-4111-8111-111111111111"
STEP_ID = "22222222-2222-4222-8222-222222222222"


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], text=True, capture_output=True, check=False)


def test_strict_dispatch_accepts_release_control_bot():
    result = run(
        "validate-dispatch",
        "--operation-id",
        OPERATION_ID,
        "--step-id",
        STEP_ID,
        "--actor",
        "iii-release-control[bot]",
        "--triggering-actor",
        "iii-release-control[bot]",
        "--expected-bot",
        "iii-release-control[bot]",
        "--run-attempt",
        "1",
        "--mutating",
    )
    assert result.returncode == 0, result.stderr


def test_strict_dispatch_rejects_user_and_rerun():
    for actor, attempt in (("human", "1"), ("iii-release-control[bot]", "2")):
        result = run(
            "validate-dispatch",
            "--operation-id",
            OPERATION_ID,
            "--step-id",
            STEP_ID,
            "--actor",
            actor,
            "--triggering-actor",
            actor,
            "--expected-bot",
            "iii-release-control[bot]",
            "--run-attempt",
            attempt,
            "--mutating",
        )
        assert result.returncode == 2


def test_release_dispatch_validates_frozen_identity_inputs():
    valid = run(
        "validate-dispatch",
        "--operation-id",
        OPERATION_ID,
        "--step-id",
        STEP_ID,
        "--actor",
        "iii-release-control[bot]",
        "--triggering-actor",
        "iii-release-control[bot]",
        "--expected-bot",
        "iii-release-control[bot]",
        "--run-attempt",
        "1",
        "--mutating",
        "--plan-hash",
        "b" * 64,
        "--dispatch-nonce",
        "66666666-6666-4666-8666-666666666666",
        "--candidate-version",
        "1.2.3-rc.1",
        "--stable-version",
        "1.2.3",
        "--source-sha",
        "a" * 40,
        "--prepared-sha",
        "c" * 40,
    )
    assert valid.returncode == 0, valid.stderr

    invalid = run(
        "validate-dispatch",
        "--operation-id",
        OPERATION_ID,
        "--step-id",
        STEP_ID,
        "--actor",
        "iii-release-control[bot]",
        "--triggering-actor",
        "iii-release-control[bot]",
        "--expected-bot",
        "iii-release-control[bot]",
        "--run-attempt",
        "1",
        "--mutating",
        "--plan-hash",
        "not-a-plan",
    )
    assert invalid.returncode == 2


def test_result_contains_only_factual_sections(tmp_path: pathlib.Path):
    output = tmp_path / "execution-result.json"
    result = run(
        "write-result",
        "--kind",
        "release",
        "--repository",
        "iii-hq/workers",
        "--operation-id",
        OPERATION_ID,
        "--step-id",
        STEP_ID,
        "--run-id",
        "123",
        "--run-attempt",
        "1",
        "--workflow",
        "release.yml",
        "--event",
        "workflow_dispatch",
        "--sha",
        "a" * 40,
        "--subject",
        '{"worker":"eval","version":"1.2.3"}',
        "--checks",
        '[{"name":"build","status":"success"}]',
        "--effects",
        '[{"surface":"registry","status":"changed"}]',
        "--outputs",
        '{"tag":"eval/v1.2.3"}',
        "--release-intent-id",
        "33333333-3333-4333-8333-333333333333",
        "--candidate-id",
        "44444444-4444-4444-8444-444444444444",
        "--attempt-id",
        "55555555-5555-4555-8555-555555555555",
        "--plan-hash",
        "b" * 64,
        "--dispatch-nonce",
        "nonce-1",
        "--candidate-version",
        "1.2.3-rc.1",
        "--stable-version",
        "1.2.3",
        "--source-sha",
        "a" * 40,
        "--prepared-sha",
        "c" * 40,
        "--digests-json",
        '[{"name":"worker.tar.gz","sha256":"d"}]',
        "--output",
        str(output),
    )
    assert result.returncode == 0, result.stderr
    payload = json.loads(output.read_text())
    assert payload["schema_version"] == 1
    assert payload["run"]["attempt"] == 1
    assert "status" not in payload
    assert "promotable" not in payload
    assert payload["release"]["candidate_id"] == "44444444-4444-4444-8444-444444444444"
    assert payload["release"]["digests"] == [{"name": "worker.tar.gz", "sha256": "d"}]
