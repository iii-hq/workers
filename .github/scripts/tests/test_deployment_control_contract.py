import hashlib
import json
import pathlib
import subprocess
import sys
from types import SimpleNamespace

import deployment_control_contract as contract


SCRIPT = pathlib.Path(__file__).parents[1] / "deployment_control_contract.py"
SCHEMA = pathlib.Path(__file__).parents[2] / "contracts" / "deployment-execution.schema.json"
DEPLOYMENT_BATCH_ID = "11111111-1111-4111-8111-111111111111"
STEP_ID = "22222222-2222-4222-8222-222222222222"
DEPLOYMENT_TARGET_ID = "44444444-4444-4444-8444-444444444444"
ATTEMPT_ID = "55555555-5555-4555-8555-555555555555"
NONCE = "66666666-6666-4666-8666-666666666666"


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], text=True, capture_output=True, check=False)


def test_contract_schema_is_byte_identical_to_release_control():
    assert hashlib.sha256(SCHEMA.read_bytes()).hexdigest() == "c9cd573a793ef8287e6d75814f4c7d474449e10dcec2238ff0d85648c6fb6784"


def test_compact_identity_input_requires_exact_canonical_fields():
    identity = {
        "deployment_batch_id": DEPLOYMENT_BATCH_ID,
        "deployment_target_id": DEPLOYMENT_TARGET_ID,
        "step_id": STEP_ID,
        "attempt_id": ATTEMPT_ID,
        "dispatch_nonce": NONCE,
        "plan_hash": "b" * 64,
    }
    assert run("validate-identity", "--identity", json.dumps(identity)).returncode == 0
    identity["unknown"] = "rejected"
    result = run("validate-identity", "--identity", json.dumps(identity))
    assert result.returncode == 2
    assert "identity fields differ" in result.stderr


def test_dispatch_identity_is_syntactic_and_reruns_are_rejected():
    common = ["validate-dispatch", "--step-id", STEP_ID,
              "--dispatch-nonce", NONCE, "--descriptor-sha256", "d" * 64,
              "--plan-hash", "b" * 64, "--source-sha", "a" * 40, "--mutating"]
    assert run(*common, "--run-attempt", "1").returncode == 0
    assert run(*common, "--run-attempt", "2").returncode == 2


def test_release_result_has_exact_nominal_shape(tmp_path: pathlib.Path):
    output = tmp_path / "deployment-result.json"
    result = run(
        "write-result", "--repository", "iii-hq/workers",
        "--step-id", STEP_ID, "--run-id", "123", "--run-attempt", "1",
        "--workflow", "deploy-prepare.yml", "--event", "workflow_dispatch", "--sha", "a" * 40,
        "--deployment-batch-id", DEPLOYMENT_BATCH_ID, "--deployment-target-id", DEPLOYMENT_TARGET_ID,
        "--attempt-id", ATTEMPT_ID,
        "--dispatch-nonce", NONCE, "--plan-hash", "b" * 64, "--worker", "eval", "--phase", "prepare",
        "--source-sha", "a" * 40, "--prepared-sha", "c" * 40,
        "--target-version", "1.2.3-beta", "--channel", "next",
        "--descriptor-sha256", "d" * 64, "--outcome", "succeeded",
        "--effects", '[{"surface":"git-ref","state":"present","immutable_id":"c"}]',
        "--artifacts-json", '[{"name":"eval.tar.gz","role":"bundle","sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","size":42}]',
        "--error-json", "null", "--output", str(output),
    )
    assert result.returncode == 0, result.stderr
    payload = json.loads(output.read_text())
    assert set(payload) == {"contract", "identity", "executor", "subject", "outcome", "effects", "artifacts", "error", "completed_at"}
    assert payload["contract"] == "deployment-execution"
    assert set(payload["identity"]) == {
        "deployment_batch_id", "deployment_target_id", "step_id",
        "attempt_id", "dispatch_nonce", "plan_hash",
    }
    assert payload["identity"]["deployment_batch_id"] == DEPLOYMENT_BATCH_ID
    assert payload["identity"]["deployment_target_id"] == DEPLOYMENT_TARGET_ID
    assert payload["subject"]["phase"] == "prepare"
    assert payload["subject"]["target_version"] == "1.2.3-beta"
    assert payload["subject"]["channel"] == "next"
    assert payload["error"] is None


class Response:
    status = 200

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False


def executor_args(**overrides):
    values = {
        "api_url": "https://release-control.example",
        "audience": "release-control-workers",
        "repository": "iii-hq/workers",
        "deployment_batch_id": DEPLOYMENT_BATCH_ID,
        "deployment_target_id": DEPLOYMENT_TARGET_ID,
        "step_id": STEP_ID,
        "run_id": 123,
        "run_attempt": 1,
        "workflow": "deploy-prepare.yml",
        "event": "workflow_dispatch",
        "sha": "a" * 40,
        "attempt_id": ATTEMPT_ID,
        "dispatch_nonce": NONCE,
        "plan_hash": "b" * 64,
        "worker": "eval",
        "source_sha": "c" * 40,
        "target_version": "1.2.3-beta",
        "channel": "next",
        "descriptor_sha256": "d" * 64,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def test_authorize_posts_nested_canonical_body_and_its_digest(monkeypatch):
    captured = {}
    monkeypatch.setattr(contract, "oidc_token", lambda audience: f"oidc:{audience}")

    def urlopen(request, timeout):
        captured.update(request=request, timeout=timeout)
        return Response()

    monkeypatch.setattr(contract.urllib.request, "urlopen", urlopen)
    assert contract.authorize_dispatch(executor_args()) == 0
    request = captured["request"]
    body = request.data
    payload = json.loads(body)
    assert request.full_url == "https://release-control.example/executor-dispatches/authorize"
    assert set(payload) == {"identity", "executor", "subject"}
    assert set(payload["identity"]) == {
        "deployment_batch_id", "deployment_target_id", "step_id",
        "attempt_id", "dispatch_nonce", "plan_hash",
    }
    assert set(payload["subject"]) == {
        "worker", "source_sha", "target_version", "channel", "descriptor_sha256"
    }
    assert request.get_header("X-deployment-result-sha256") == "sha256:" + hashlib.sha256(body).hexdigest()
    assert request.get_header("Authorization") == "Bearer oidc:release-control-workers"


def test_new_deployment_target_accepts_numbered_rc_on_next(tmp_path: pathlib.Path):
    output = tmp_path / "deployment-result.json"
    result = run(
        "write-result", "--repository", "iii-hq/workers", "--step-id", STEP_ID,
        "--run-id", "123", "--run-attempt", "1", "--workflow", "deploy-prepare.yml",
        "--event", "workflow_dispatch", "--sha", "a" * 40,
        "--deployment-batch-id", DEPLOYMENT_BATCH_ID,
        "--deployment-target-id", DEPLOYMENT_TARGET_ID, "--attempt-id", ATTEMPT_ID,
        "--dispatch-nonce", NONCE, "--plan-hash", "b" * 64, "--worker", "eval",
        "--phase", "prepare", "--source-sha", "a" * 40,
        "--target-version", "1.2.3-rc.1", "--channel", "next",
        "--descriptor-sha256", "d" * 64, "--outcome", "succeeded", "--effects", "[]",
        "--artifacts-json", "[]", "--error-json", "null",
        "--output", str(output),
    )
    assert result.returncode == 0, result.stderr
    payload = json.loads(output.read_text())
    assert payload["subject"]["target_version"] == "1.2.3-rc.1"


def test_post_result_sends_the_file_bytes_without_reserializing(tmp_path, monkeypatch):
    body = b'{"contract":"deployment-execution", "space":"is preserved"}\n'
    result = tmp_path / "deployment-result.json"
    result.write_bytes(body)
    captured = {}
    monkeypatch.setattr(contract, "oidc_token", lambda _audience: "oidc")

    def urlopen(request, timeout):
        captured.update(request=request, timeout=timeout)
        return Response()

    monkeypatch.setattr(contract.urllib.request, "urlopen", urlopen)
    args = SimpleNamespace(result=result, api_url="https://release-control.example", audience="release-control-workers")
    assert contract.post_result(args) == 0
    request = captured["request"]
    assert request.full_url == "https://release-control.example/executor-results"
    assert request.data == body
    assert request.get_header("X-deployment-result-sha256") == "sha256:" + hashlib.sha256(body).hexdigest()


def test_phase_and_workflow_must_match(tmp_path: pathlib.Path):
    result = run(
        "write-result", "--repository", "iii-hq/workers",
        "--step-id", STEP_ID, "--run-id", "123", "--run-attempt", "1",
        "--workflow", "deploy-verify.yml", "--event", "workflow_dispatch", "--sha", "a" * 40,
        "--deployment-batch-id", DEPLOYMENT_BATCH_ID, "--deployment-target-id", DEPLOYMENT_TARGET_ID,
        "--attempt-id", ATTEMPT_ID,
        "--dispatch-nonce", NONCE, "--plan-hash", "b" * 64, "--worker", "eval", "--phase", "prepare",
        "--source-sha", "a" * 40, "--target-version", "1.2.3", "--channel", "latest",
        "--descriptor-sha256", "d" * 64, "--outcome", "failed",
        "--effects", "[]", "--artifacts-json", "[]",
        "--error-json", '{"code":"x","category":"test","retryable":false,"message":"x"}',
        "--output", str(tmp_path / "deployment-result.json"),
    )
    assert result.returncode == 2
    assert "requires workflow deploy-prepare.yml" in result.stderr


def run_finalize_write_result(output: pathlib.Path, **overrides: str) -> subprocess.CompletedProcess[str]:
    values = {
        "repository": "iii-hq/workers",
        "step-id": STEP_ID,
        "run-id": "123",
        "run-attempt": "1",
        "workflow": "deploy-finalize.yml",
        "event": "workflow_dispatch",
        "sha": "a" * 40,
        "deployment-batch-id": DEPLOYMENT_BATCH_ID,
        "deployment-target-id": DEPLOYMENT_TARGET_ID,
        "attempt-id": ATTEMPT_ID,
        "dispatch-nonce": NONCE,
        "plan-hash": "b" * 64,
        "worker": "eval",
        "phase": "finalize",
        "source-sha": "a" * 40,
        "prepared-sha": "a" * 40,
        "candidate-version": "1.7.0-rc.2",
        "target-version": "1.7.0",
        "channel": "latest",
        "descriptor-sha256": "d" * 64,
        "outcome": "succeeded",
        "effects": "[]",
        "artifacts-json": "[]",
        "error-json": "null",
        "output": str(output),
    }
    values.update({key.replace("_", "-"): value for key, value in overrides.items()})
    return run("write-result", *[part for key, value in values.items() for part in (f"--{key}", value)])


def test_finalize_result_binds_candidate_to_its_stable_version(tmp_path: pathlib.Path):
    output = tmp_path / "deployment-result.json"
    result = run_finalize_write_result(output)
    assert result.returncode == 0, result.stderr
    payload = json.loads(output.read_text())
    assert set(payload) == {"contract", "identity", "executor", "subject", "outcome", "effects", "artifacts", "error", "completed_at"}
    assert set(payload["subject"]) == {
        "worker", "phase", "source_sha", "prepared_sha", "target_version",
        "channel", "descriptor_sha256", "candidate_version",
    }
    assert payload["subject"]["phase"] == "finalize"
    assert payload["subject"]["channel"] == "latest"
    assert payload["subject"]["target_version"] == "1.7.0"
    assert payload["subject"]["candidate_version"] == "1.7.0-rc.2"
    assert payload["subject"]["prepared_sha"] == payload["subject"]["source_sha"]


def test_finalize_requires_a_candidate_version(tmp_path: pathlib.Path):
    result = run_finalize_write_result(tmp_path / "deployment-result.json", candidate_version="none")
    assert result.returncode == 2
    assert "require --candidate-version" in result.stderr


def test_finalize_requires_channel_latest(tmp_path: pathlib.Path):
    result = run_finalize_write_result(tmp_path / "deployment-result.json", channel="next")
    assert result.returncode == 2
    assert "require channel=latest" in result.stderr


def test_finalize_requires_prepared_sha_equal_to_source_sha(tmp_path: pathlib.Path):
    result = run_finalize_write_result(tmp_path / "deployment-result.json", prepared_sha="c" * 40)
    assert result.returncode == 2
    assert "prepared_sha equal to source_sha" in result.stderr


def test_non_finalize_phases_reject_a_candidate_version(tmp_path: pathlib.Path):
    result = run_finalize_write_result(
        tmp_path / "deployment-result.json",
        workflow="deploy-prepare.yml", phase="prepare", channel="next",
        target_version="1.7.0-rc.2",
    )
    assert result.returncode == 2
    assert "does not accept" in result.stderr


def test_latest_channel_rejects_rc_target_versions(tmp_path: pathlib.Path):
    result = run_finalize_write_result(
        tmp_path / "deployment-result.json",
        workflow="deploy-publish.yml", phase="publish",
        candidate_version="none", target_version="1.2.3-rc.1",
    )
    assert result.returncode == 2
    assert "pure MAJOR.MINOR.PATCH" in result.stderr


def test_finalize_phase_requires_the_finalize_workflow(tmp_path: pathlib.Path):
    result = run_finalize_write_result(tmp_path / "deployment-result.json", workflow="deploy-verify.yml")
    assert result.returncode == 2
    assert "requires workflow deploy-finalize.yml" in result.stderr
