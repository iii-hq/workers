import hashlib
import json
import pathlib
import subprocess
import sys
from types import SimpleNamespace

import release_control_contract as contract


SCRIPT = pathlib.Path(__file__).parents[1] / "release_control_contract.py"
SCHEMA = pathlib.Path(__file__).parents[2] / "contracts" / "release-execution.schema.json"
OPERATION_ID = "11111111-1111-4111-8111-111111111111"
STEP_ID = "22222222-2222-4222-8222-222222222222"
INTENT_ID = "33333333-3333-4333-8333-333333333333"
CANDIDATE_ID = "44444444-4444-4444-8444-444444444444"
ATTEMPT_ID = "55555555-5555-4555-8555-555555555555"
NONCE = "66666666-6666-4666-8666-666666666666"


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], text=True, capture_output=True, check=False)


def test_contract_schema_is_byte_identical_to_release_control():
    assert hashlib.sha256(SCHEMA.read_bytes()).hexdigest() == "947ccf41d51918901d994a484c5c0efaac7e8966310627354dccbb37e53733fa"


def test_compact_identity_input_requires_exact_canonical_fields():
    identity = {
        "operation_id": OPERATION_ID,
        "step_id": STEP_ID,
        "release_intent_id": INTENT_ID,
        "candidate_id": CANDIDATE_ID,
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
    common = ["validate-dispatch", "--operation-id", OPERATION_ID, "--step-id", STEP_ID,
              "--dispatch-nonce", NONCE, "--descriptor-sha256", "d" * 64,
              "--plan-hash", "b" * 64, "--source-sha", "a" * 40, "--mutating"]
    assert run(*common, "--run-attempt", "1").returncode == 0
    assert run(*common, "--run-attempt", "2").returncode == 2


def test_release_result_has_exact_nominal_shape(tmp_path: pathlib.Path):
    output = tmp_path / "release-result.json"
    result = run(
        "write-result", "--repository", "iii-hq/workers", "--operation-id", OPERATION_ID,
        "--step-id", STEP_ID, "--run-id", "123", "--run-attempt", "1",
        "--workflow", "release-prepare.yml", "--event", "workflow_dispatch", "--sha", "a" * 40,
        "--release-intent-id", INTENT_ID, "--candidate-id", CANDIDATE_ID, "--attempt-id", ATTEMPT_ID,
        "--dispatch-nonce", NONCE, "--plan-hash", "b" * 64, "--worker", "eval", "--phase", "prepare",
        "--source-sha", "a" * 40, "--prepared-sha", "c" * 40,
        "--candidate-version", "1.2.3-rc.1", "--stable-version", "1.2.3",
        "--descriptor-sha256", "d" * 64, "--outcome", "succeeded",
        "--effects", '[{"surface":"git-ref","state":"present","immutable_id":"c"}]',
        "--artifacts-json", '[{"name":"eval.tar.gz","role":"bundle","sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","size":42}]',
        "--error-json", "null", "--output", str(output),
    )
    assert result.returncode == 0, result.stderr
    payload = json.loads(output.read_text())
    assert set(payload) == {"contract", "identity", "executor", "subject", "outcome", "effects", "artifacts", "error", "completed_at"}
    assert payload["contract"] == "release-execution"
    assert payload["identity"]["candidate_id"] == CANDIDATE_ID
    assert payload["subject"]["phase"] == "prepare"
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
        "operation_id": OPERATION_ID,
        "step_id": STEP_ID,
        "run_id": 123,
        "run_attempt": 1,
        "workflow": "release-prepare.yml",
        "event": "workflow_dispatch",
        "sha": "a" * 40,
        "release_intent_id": INTENT_ID,
        "candidate_id": CANDIDATE_ID,
        "attempt_id": ATTEMPT_ID,
        "dispatch_nonce": NONCE,
        "plan_hash": "b" * 64,
        "worker": "eval",
        "source_sha": "c" * 40,
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
    assert set(payload["subject"]) == {"worker", "source_sha", "descriptor_sha256"}
    assert request.get_header("X-release-result-sha256") == "sha256:" + hashlib.sha256(body).hexdigest()
    assert request.get_header("Authorization") == "Bearer oidc:release-control-workers"


def test_post_result_sends_the_file_bytes_without_reserializing(tmp_path, monkeypatch):
    body = b'{"contract":"release-execution", "space":"is preserved"}\n'
    result = tmp_path / "release-result.json"
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
    assert request.get_header("X-release-result-sha256") == "sha256:" + hashlib.sha256(body).hexdigest()


def test_phase_and_workflow_must_match(tmp_path: pathlib.Path):
    result = run(
        "write-result", "--repository", "iii-hq/workers", "--operation-id", OPERATION_ID,
        "--step-id", STEP_ID, "--run-id", "123", "--run-attempt", "1",
        "--workflow", "release-verify.yml", "--event", "workflow_dispatch", "--sha", "a" * 40,
        "--release-intent-id", INTENT_ID, "--candidate-id", CANDIDATE_ID, "--attempt-id", ATTEMPT_ID,
        "--dispatch-nonce", NONCE, "--plan-hash", "b" * 64, "--worker", "eval", "--phase", "prepare",
        "--source-sha", "a" * 40, "--descriptor-sha256", "d" * 64, "--outcome", "failed",
        "--effects", "[]", "--artifacts-json", "[]",
        "--error-json", '{"code":"x","category":"test","retryable":false,"message":"x"}',
        "--output", str(tmp_path / "release-result.json"),
    )
    assert result.returncode == 2
    assert "requires workflow release-prepare.yml" in result.stderr
