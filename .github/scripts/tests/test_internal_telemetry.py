"""Internal launch boundaries must opt out before any III process starts."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess

import pytest
import yaml

import collect_worker_interface
import registry_worker_smoke


REPOSITORY = Path(__file__).resolve().parents[3]
WORKFLOWS = (
    "ci.yml", "build.yml", "_deploy-build.yml", "_harness-integration.yml",
    "_worker-e2e.yml", "harness-quickstart.yml", "browser-scrapling-e2e.yml",
    "database-e2e.yml", "ide-e2e.yml", "rbac-proxy-e2e.yml", "storage-e2e.yml",
)


@pytest.mark.parametrize("runner", ["interface", "registry"])
def test_cli_helpers_opt_out_without_workflow_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, runner: str,
) -> None:
    iii = tmp_path / "iii"
    iii.write_text(
        '#!/bin/sh\n'
        'exec /bin/sh -c \'printf "{\\"telemetry\\":\\"%s\\"}" "$III_TELEMETRY_ENABLED"\'\n'
    )
    iii.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path))
    monkeypatch.setenv("III_TELEMETRY_ENABLED", "true")
    monkeypatch.delenv("III_TRIGGER_PORT", raising=False)
    if runner == "interface":
        result = collect_worker_interface.run_iii("engine::workers::list", {})
    else:
        result, error = registry_worker_smoke.trigger("probe", "compose::up", {})
        assert error is None
    assert result == {"telemetry": "false"}


@pytest.mark.parametrize("name", WORKFLOWS)
def test_workflow_steps_including_installers_opt_out(name: str) -> None:
    workflow = yaml.safe_load((REPOSITORY / ".github/workflows" / name).read_text())
    # Reusable workflows need their own env: caller workflow env is not inherited.
    assert workflow.get("env", {}).get("III_TELEMETRY_ENABLED") == "false"
    for job in workflow["jobs"].values():
        for step in job.get("steps", []):
            env = {**workflow["env"], **job.get("env", {}), **step.get("env", {})}
            assert env["III_TELEMETRY_ENABLED"] == "false"


def test_oci_capture_overrides_image_environment_without_changing_otel(tmp_path: Path) -> None:
    workflow = yaml.safe_load((REPOSITORY / ".github/workflows/build.yml").read_text())
    run = next(
        step["run"] for step in workflow["jobs"]["assemble"]["steps"]
        if "docker run --rm" in step.get("run", "")
    )
    # Execute the actual launch fragment; the Docker double only reports the
    # effective container env, including the last-value-wins CLI semantics.
    launch = run[run.index("docker run --rm"):run.index("worker_pid=$!") + len("worker_pid=$!")]
    (tmp_path / "interface-work").mkdir()
    script = """
set -euo pipefail
container_name=probe
image_name=probe-image
docker_env=(--env III_TELEMETRY_ENABLED=true --env OTEL_SERVICE_NAME=interface-probe)
docker() {
    local telemetry=unset otel=unset
    while (($#)); do
        case "$1" in
            --env)
                shift
                case "$1" in
                    III_TELEMETRY_ENABLED=*) telemetry="${1#*=}" ;;
                    OTEL_SERVICE_NAME=*) otel="${1#*=}" ;;
                esac
                ;;
        esac
        shift
    done
    printf '%s:%s' "$telemetry" "$otel"
}
""" + launch + '\nwait "$worker_pid"\n'
    subprocess.run(["bash", "-c", script], cwd=tmp_path, check=True, timeout=10)
    assert (tmp_path / "interface-work/worker.log").read_text() == "false:interface-probe"


@pytest.mark.parametrize("parent_value", [None, "true"])
def test_local_cargo_process_and_nested_command_opt_out(
    tmp_path: Path, parent_value: str | None,
) -> None:
    cargo = shutil.which("cargo")
    if cargo is None:
        pytest.skip("cargo is required for the local process boundary regression")
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "telemetry-boundary-probe"\n'
        'version = "0.0.0"\nedition = "2021"\n[workspace]\n'
    )
    (tmp_path / "src").mkdir()
    (tmp_path / "src/main.rs").write_text(r"""
fn main() {
    println!("{}", std::env::var("III_TELEMETRY_ENABLED").unwrap_or_default());
    let out = std::process::Command::new("sh")
        .args(["-c", "printf %s \"$III_TELEMETRY_ENABLED\""]).output().unwrap();
    print!("{}", String::from_utf8(out.stdout).unwrap());
}
""")
    env = os.environ.copy()
    env.pop("III_TELEMETRY_ENABLED", None)
    if parent_value is not None:
        env["III_TELEMETRY_ENABLED"] = parent_value
    # Only a tiny, dependency-free environment probe executes here.
    result = subprocess.run(
        [cargo, "run", "--quiet", "--offline", "--manifest-path", str(tmp_path / "Cargo.toml")],
        cwd=REPOSITORY / "workers-dev", env=env, text=True, capture_output=True,
        check=True, timeout=60,
    )
    assert result.stdout == "false\nfalse"
