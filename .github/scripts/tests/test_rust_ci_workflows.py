from __future__ import annotations

from pathlib import Path
import re
import tomllib

import yaml


GITHUB = Path(__file__).parents[2]
REPOSITORY = GITHUB.parent
WORKFLOWS = GITHUB / "workflows"


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
    assert set(workflow_toolchains) == {"1.97.1"}


def test_prs_restore_rust_caches_and_main_pushes_publish_them() -> None:
    ci = workflow("ci.yml")
    assert ci["on"]["push"]["branches"] == ["main"]

    rust_cache = next(
        step for step in ci["jobs"]["rust"]["steps"]
        if step.get("uses") == "Swatinem/rust-cache@v2"
    )
    crate_cache = next(
        step for step in ci["jobs"]["crates"]["steps"]
        if step.get("uses") == "Swatinem/rust-cache@v2"
    )
    expected = "${{ github.event_name == 'push' }}"
    assert rust_cache["with"]["save-if"] == expected
    assert crate_cache["with"]["save-if"] == expected
    assert "github.event_name == 'pull_request'" in ci["jobs"]["interface-smoke"]["if"]
    assert ci["jobs"]["harness-integration"]["with"]["save-cache"] == expected


def test_harness_integration_uses_the_rust_self_hosted_runner() -> None:
    integration = workflow("_harness-integration.yml")["jobs"]["integration"]
    assert integration["runs-on"] == ["self-hosted", "Linux", "X64", "rust"]


def test_actionlint_knows_the_repository_runner_pool_labels() -> None:
    config = yaml.load(
        (GITHUB / "actionlint.yaml").read_text(), Loader=yaml.BaseLoader
    )
    assert config["self-hosted-runner"]["labels"] == ["general", "rust"]


def test_interface_smoke_bounds_each_engine_readiness_probe() -> None:
    steps = workflow("ci.yml")["jobs"]["interface-smoke"]["steps"]
    run = named_step(steps, "Start III engine")["run"]

    assert "deadline=$((SECONDS + 60))" in run
    assert "while (( SECONDS < deadline )); do" in run
    assert (
        "timeout --signal=KILL 5s iii trigger 'engine::workers::list' "
        "--json '{}' >/dev/null 2>&1"
    ) in run


def test_harness_integration_caches_the_final_engine_and_skips_rebuilds() -> None:
    integration = workflow("_harness-integration.yml")
    steps = integration["jobs"]["integration"]["steps"]
    restore = named_step(steps, "Restore pinned engine binary")
    build = named_step(steps, "Build pinned engine")
    save = named_step(steps, "Save pinned engine binary")
    stack_cache = named_step(steps, "Restore integration Rust cache")

    expected_path = "target/integration-engine-src/${{ steps.lock.outputs.binary }}"
    assert restore["with"]["path"] == expected_path
    assert "integration-engine-bin-rust-1.97.1" in restore["with"]["key"]
    assert "hashFiles('harness/tests/integration/engine.lock')" in restore["with"]["key"]
    assert build["if"] == "steps.engine-cache.outputs.cache-hit != 'true'"
    assert "--locked --release --timings" in build["run"]
    assert save["with"]["path"] == expected_path
    assert "database -> target" in stack_cache["with"]["workspaces"]


def test_slow_rust_builds_upload_cargo_timing_reports() -> None:
    integration = workflow("_harness-integration.yml")
    integration_steps = integration["jobs"]["integration"]["steps"]
    timing_upload = named_step(integration_steps, "Upload Rust build timings")
    assert timing_upload["if"] == "always()"
    assert "cargo-timings/*.html" in timing_upload["with"]["path"]

    e2e = workflow("_harness-e2e.yml")
    e2e_steps = e2e["jobs"]["build"]["steps"]
    e2e_cache = next(
        step for step in e2e_steps if step.get("uses") == "Swatinem/rust-cache@v2"
    )
    timing_upload = named_step(e2e_steps, "Upload Rust build timings")
    assert timing_upload["if"] == "always()"
    assert "--timings" in named_step(e2e_steps, "Build source E2E stack")["run"]
    assert "fp -> target" in e2e_cache["with"]["workspaces"]


def test_ci_cargo_commands_use_committed_lockfiles() -> None:
    ci_body = (WORKFLOWS / "ci.yml").read_text()
    integration_body = (WORKFLOWS / "_harness-integration.yml").read_text()
    release = workflow("_rust-binary.yml")
    release_steps = release["jobs"]["build"]["steps"]
    upload = named_step(release_steps, "Build and upload binary")

    assert "cargo clippy --locked --all-targets --all-features" in ci_body
    assert "cargo test --locked --all-features" in ci_body
    assert "cargo build --locked" in ci_body
    assert "cargo test --locked --manifest-path harness/Cargo.toml" in integration_body
    assert upload["with"]["locked"] == "true"


def test_rust_security_audit_is_narrow_on_prs_and_complete_on_schedule() -> None:
    audit = workflow("rust-security-audit.yml")
    assert "workflow_dispatch" not in audit["on"]
    assert audit["on"]["pull_request"]["paths"] == ["**/Cargo.lock"]
    assert audit["on"]["schedule"]

    steps = audit["jobs"]["audit"]["steps"]
    install = named_step(steps, "Install cargo-audit")
    run = named_step(steps, "Audit Rust lockfiles")["run"]
    assert install["uses"] == "taiki-e/install-action@v2.85.13"
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
