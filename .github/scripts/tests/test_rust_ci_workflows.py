from __future__ import annotations

from pathlib import Path
import re
import tomllib

import yaml


GITHUB = Path(__file__).parents[2]
REPOSITORY = GITHUB.parent
WORKFLOWS = GITHUB / "workflows"
RUST_CACHE_ACTION = "Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6"


def workflow(name: str) -> dict:
    value = yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)
    assert isinstance(value, dict)
    return value


def named_step(steps: list[dict], name: str) -> dict:
    return next(step for step in steps if step.get("name") == name)


def test_rust_toolchain_is_pinned_to_the_last_verified_stable() -> None:
    toolchain = tomllib.loads((REPOSITORY / "rust-toolchain.toml").read_text())
    assert toolchain["toolchain"]["channel"] == "1.97.1"

    bodies = "\n".join(path.read_text() for path in WORKFLOWS.glob("*.yml"))
    workflow_toolchains = re.findall(r"dtolnay/rust-toolchain@([^\s]+)", bodies)
    assert workflow_toolchains
    assert set(workflow_toolchains) == {"6c977a6ca4077a0ceb28ffbe03f59d46e9ac8772"}
    assert "toolchain: 1.97.1" in bodies
    assert "toolchain: ${{ steps.metadata.outputs.toolchain_version }}" in (
        WORKFLOWS / "_deploy-build.yml"
    ).read_text()


def test_all_external_actions_are_pinned_by_commit_sha() -> None:
    for path in sorted(WORKFLOWS.glob("*.yml")):
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            match = re.search(r"\buses:\s*([^\s#]+)", line)
            if not match or match.group(1).startswith("./"):
                continue
            reference = match.group(1)
            assert re.fullmatch(r"[^@]+@[0-9a-f]{40}", reference), (
                f"{path.name}:{line_number}: mutable action reference {reference}"
            )


def test_prs_restore_rust_caches_and_main_pushes_publish_them() -> None:
    ci = workflow("ci.yml")
    assert ci["on"]["push"]["branches"] == ["main"]

    rust_cache = next(
        step for step in ci["jobs"]["rust"]["steps"]
        if step.get("uses") == RUST_CACHE_ACTION
    )
    crate_cache = next(
        step for step in ci["jobs"]["crates"]["steps"]
        if step.get("uses") == RUST_CACHE_ACTION
    )
    expected = "${{ github.event_name == 'push' }}"
    assert rust_cache["with"]["save-if"] == expected
    assert crate_cache["with"]["save-if"] == expected
    assert "github.event_name == 'pull_request'" in ci["jobs"]["interface-smoke"]["if"]
    assert ci["jobs"]["harness-integration"]["with"]["save-cache"] == expected


def test_harness_integration_defaults_to_an_available_hosted_runner() -> None:
    reusable = workflow("_harness-integration.yml")
    assert reusable["on"]["workflow_call"]["inputs"]["runner"]["default"] == (
        "ubuntu-latest"
    )
    assert reusable["jobs"]["integration"]["runs-on"] == "${{ inputs.runner }}"
    assert reusable["jobs"]["integration"]["timeout-minutes"] == "90"


def test_harness_benchmark_is_manual_and_keeps_cold_caches_isolated() -> None:
    benchmark = workflow("harness-integration-benchmark.yml")
    inputs = benchmark["on"]["workflow_dispatch"]["inputs"]
    assert inputs["runner"]["options"] == [
        "workers-ci-linux-8core",
        "ubuntu-latest",
    ]
    assert inputs["cache-mode"]["options"] == ["warm", "cold"]

    call = benchmark["jobs"]["integration"]
    assert call["uses"] == "./.github/workflows/_harness-integration.yml"
    assert call["with"]["runner"] == "${{ inputs.runner }}"
    assert call["with"]["save-cache"] == "${{ inputs.cache-mode == 'warm' }}"
    assert "github.run_id" in call["with"]["cache-key-suffix"]

    reusable_body = (WORKFLOWS / "_harness-integration.yml").read_text()
    assert "format('-{0}', inputs.cache-key-suffix)" in reusable_body


def test_actionlint_knows_the_repository_runner_pool_labels() -> None:
    config = yaml.load(
        (GITHUB / "actionlint.yaml").read_text(), Loader=yaml.BaseLoader
    )
    assert config["self-hosted-runner"]["labels"] == [
        "general",
        "workers-ci-linux-8core",
        "workers-release-control-linux-2core",
        "workers-release-linux-8core",
        "workers-release-macos-aws-intel",
        "workers-release-macos-12core",
        "workers-release-macos-arm-5core",
    ]


def test_runner_pool_contract_is_documented() -> None:
    runner_docs = (REPOSITORY / "docs/architecture/testing-and-ci.md").read_text()
    labels = {
        "ubuntu-latest",
        "workers-ci-linux-8core",
        "workers-release-control-linux-2core",
        "workers-release-linux-8core",
        "windows-latest",
        "workers-release-macos-12core",
        "workers-release-macos-arm-5core",
        "workers-release-macos-aws-intel",
    }
    for label in labels:
        assert f"`{label}`" in runner_docs

    assert "ten candidate executions" in runner_docs
    assert "improvement over `ubuntu-latest` is at least 25%" in runner_docs
    assert "8-core warm | 7 | 8m54 | 9m17" in runner_docs
    assert "`ubuntu-latest` remains the default" in runner_docs


def test_interface_smoke_bounds_each_engine_readiness_probe() -> None:
    steps = workflow("ci.yml")["jobs"]["interface-smoke"]["steps"]
    run = named_step(steps, "Start III engine")["run"]

    assert "deadline=$((SECONDS + 60))" in run
    assert "while (( SECONDS < deadline )); do" in run
    assert (
        "timeout --signal=KILL 5s iii trigger 'engine::workers::list' "
        "--json '{}' >/dev/null 2>&1"
    ) in run


def test_harness_integration_downloads_latest_rc_engine_without_building_it() -> None:
    integration = workflow("_harness-integration.yml")
    steps = integration["jobs"]["integration"]["steps"]
    install = named_step(steps, "Install latest iii @rc")
    stack_cache = named_step(steps, "Restore integration Rust cache")

    assert "https://install.iii.dev/iii/main/install.sh" in install["run"]
    assert 'sh "$installer" --rc' in install["run"]
    assert "command -v iii" in install["run"]
    assert "III_ENGINE_VERSION" in install["run"]
    assert "Build pinned engine" not in {step.get("name") for step in steps}
    assert "integration-engine-src" not in (WORKFLOWS / "_harness-integration.yml").read_text()
    assert "cron -> target" in stack_cache["with"]["workspaces"]
    assert "database -> target" in stack_cache["with"]["workspaces"]


def test_slow_rust_builds_upload_cargo_timing_reports() -> None:
    integration = workflow("_harness-integration.yml")
    integration_steps = integration["jobs"]["integration"]["steps"]
    timing_upload = named_step(integration_steps, "Upload Rust build timings")
    assert timing_upload["if"] == "always()"
    assert "cargo-timings/*.html" in timing_upload["with"]["path"]

    e2e = workflow("_worker-e2e.yml")
    e2e_steps = e2e["jobs"]["build"]["steps"]
    e2e_cache = next(
        step for step in e2e_steps if str(step.get("uses", "")).startswith("Swatinem/rust-cache@")
    )
    timing_upload = named_step(e2e_steps, "Upload Rust build timings")
    assert timing_upload["if"] == "always()"
    assert "--timings" in named_step(e2e_steps, "Build source E2E stack")["run"]
    assert "fp -> target" in e2e_cache["with"]["workspaces"]


def test_ci_cargo_commands_use_committed_lockfiles() -> None:
    ci_body = (WORKFLOWS / "ci.yml").read_text()
    integration_body = (WORKFLOWS / "_harness-integration.yml").read_text()
    release = workflow("_deploy-build.yml")
    release_steps = release["jobs"]["build"]["steps"]
    build = named_step(release_steps, "Build immutable artifact")

    assert "cargo clippy --locked --all-targets --all-features" in ci_body
    assert "cargo test --locked --all-features" in ci_body
    assert "cargo build --locked" in ci_body
    assert "cargo test --locked --manifest-path harness/Cargo.toml" in integration_body
    assert "deployment_train.py build" in build["run"]


def test_rust_security_audit_is_narrow_on_prs_and_complete_on_schedule() -> None:
    audit = workflow("rust-security-audit.yml")
    assert "workflow_dispatch" not in audit["on"]
    assert audit["on"]["pull_request"]["paths"] == ["**/Cargo.lock"]
    assert audit["on"]["schedule"]

    steps = audit["jobs"]["audit"]["steps"]
    install = named_step(steps, "Install cargo-audit")
    run = named_step(steps, "Audit Rust lockfiles")["run"]
    assert install["uses"] == (
        "taiki-e/install-action@82cd3e7658a6f96c86c0234aeeda1748937cb0a1"
    )
    assert install["with"]["tool"] == "cargo-audit@0.22.2"
    assert "git diff --name-only -z" in run
    assert "find . -name Cargo.lock" in run
    assert "cargo audit --file" in run
    # Every iteration fetches: the yanked check resolves crates against the
    # index entries fetched by its own invocation, so a no-fetch pass only
    # sees whatever the first lockfile happened to warm — each additional
    # lockfile in a PR then fails its yanked lookups. (The workflow's comment
    # may name the flag; only the invocation form is forbidden.)
    assert "cargo audit --no-fetch" not in run
