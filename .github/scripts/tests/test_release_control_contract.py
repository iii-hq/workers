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
        "--output",
        str(output),
    )
    assert result.returncode == 0, result.stderr
    payload = json.loads(output.read_text())
    assert payload["schema_version"] == 1
    assert payload["run"]["attempt"] == 1
    assert "status" not in payload
    assert "promotable" not in payload
