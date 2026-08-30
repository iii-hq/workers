import re
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = ROOT / ".github" / "workflows" / "harness-quickstart.yml"


def body() -> str:
    return WORKFLOW.read_text(encoding="utf-8")


def workflow() -> dict:
    return yaml.safe_load(body())


def test_quickstart_workflow_exists_with_exact_dispatch_identity():
    document = workflow()
    inputs = document[True]["workflow_dispatch"]["inputs"]
    assert set(inputs) == {"quickstart_id", "attempt", "cli_channel", "registry_tag"}
    assert all(inputs[name]["required"] for name in inputs)
    assert inputs["cli_channel"]["options"] == ["latest", "rc"]
    assert inputs["registry_tag"]["options"] == ["latest", "next"]
    # Release Control correlates runs by this exact title shape.
    assert (
        document["run-name"]
        == "RC · quickstart · ${{ inputs.quickstart_id }} · ${{ inputs.attempt }}"
    )


def test_quickstart_job_runs_the_validator_within_its_timeout():
    document = workflow()
    jobs = document["jobs"]
    assert set(jobs) == {"quickstart"}
    job = jobs["quickstart"]
    assert job["timeout-minutes"] == 20
    validator_steps = [
        step for step in job["steps"] if step.get("run") == "harness/tests/quickstart/run-ci.sh"
    ]
    assert len(validator_steps) == 1
    assert validator_steps[0]["env"]["III_CLI_CHANNEL"] == "${{ inputs.cli_channel }}"
    assert validator_steps[0]["env"]["III_WORKER_TAG"] == "${{ inputs.registry_tag }}"


def test_quickstart_validates_the_dispatching_actor_before_any_checkout():
    steps = workflow()["jobs"]["quickstart"]["steps"]
    guard = steps[0]
    assert "RELEASE_CONTROL_BOT_LOGIN" in str(guard.get("env", {}))
    assert 'test "$ACTOR" = "$BOT_LOGIN"' in guard["run"]
    assert all("checkout" not in str(step.get("uses", "")) for step in [guard])


def test_quickstart_result_artifact_always_uploads_the_identity_named_bundle():
    steps = workflow()["jobs"]["quickstart"]["steps"]
    uploads = [step for step in steps if str(step.get("uses", "")).startswith("actions/upload-artifact@")]
    assert len(uploads) == 1
    upload = uploads[0]
    assert upload["if"] == "always()"
    assert (
        upload["with"]["name"]
        == "quickstart-result-${{ inputs.quickstart_id }}-${{ inputs.attempt }}"
    )
    assert upload["with"]["path"] == "target/harness-quickstart/"
    assert upload["with"]["if-no-files-found"] == "warn"


def test_quickstart_actions_are_sha_pinned_and_inputs_never_reach_shell_source():
    for match in re.finditer(r"uses:\s*(\S+)", body()):
        reference = match.group(1)
        assert re.search(r"@[0-9a-f]{40}$", reference), reference
    for step in workflow()["jobs"]["quickstart"]["steps"]:
        run = step.get("run")
        if run is None:
            continue
        assert "${{ inputs." not in run, "dispatch inputs must reach run: only through env:"
