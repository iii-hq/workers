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
    assert config["self-hosted-runner"]["labels"] == [
        "general",
        "rust",
        "workers-release-linux-8core",
        "workers-release-macos-12core",
        "workers-release-macos-arm-5core",
    ]


def test_interface_smoke_bounds_each_engine_readiness_probe() -> None:
    steps = workflow("ci.yml")["jobs"]["interface-smoke"]["steps"]
    run = named_step(steps, "Start III engine")["run"]

    assert "deadline=$((SECONDS + 60))" in run
    assert "while (( SECONDS < deadline )); do" in run
    assert (
        "timeout --signal=KILL 5s iii trigger 'engine::workers::list' "
        "--json '{}' >/dev/null 2>&1"
    ) in run


def test_harness_integration_keeps_scenarios_on_the_source_pinned_engine() -> None:
    integration = workflow("_harness-integration.yml")
    steps = integration["jobs"]["integration"]["steps"]
    restore = named_step(steps, "Restore pinned engine binary")
    build = named_step(steps, "Build pinned engine")
    save = named_step(steps, "Save pinned engine binary")
    lock = tomllib.loads(
        (REPOSITORY / "harness" / "tests" / "integration" / "engine.lock").read_text()
    )

    assert lock["revision"] == "15dc993ebfdbfcabe5d299cf4cae4dd676db4c06"
    expected_path = "target/integration-engine-src/${{ steps.lock.outputs.binary }}"
    assert restore["with"]["path"] == expected_path
    assert "integration-engine-bin-rust-1.97.1" in restore["with"]["key"]
    assert "hashFiles('harness/tests/integration/engine.lock')" in restore["with"]["key"]
    assert named_step(steps, "Checkout pinned engine source")["with"]["ref"] == (
        "${{ steps.lock.outputs.revision }}"
    )
    assert build["if"] == "steps.engine-cache.outputs.cache-hit != 'true'"
    assert "--locked --release --timings" in build["run"]
    assert save["with"]["path"] == expected_path
    assert named_step(steps, "Run integration scenarios")["run"].endswith(
        'III_BIN="${{ steps.engine.outputs.bin }}"'
    )


def test_harness_integration_installs_checksum_pinned_compose_release() -> None:
    integration = workflow("_harness-integration.yml")
    steps = integration["jobs"]["integration"]["steps"]
    install = named_step(steps, "Install pinned Compose release")
    stack_cache = named_step(steps, "Restore fallback integration Rust cache")
    lock = tomllib.loads(
        (REPOSITORY / "harness" / "tests" / "integration" / "compose.lock").read_text()
    )

    assert lock["tag"] == "iii/v0.23.0-rc.5"
    assert lock["revision"] == "42b8627dd697076a339f8a1318843fae1f38a283"
    assert lock["x86_64_asset"].endswith("unknown-linux-musl.tar.gz")
    assert len(lock["x86_64_sha256"]) == 64
    assert len(lock["aarch64_sha256"]) == 64
    assert "--proto '=https' --tlsv1.2" in install["run"]
    assert "--location --proto-redir '=https'" in install["run"]
    assert "sha256sum --check" in install["run"]
    assert '[[ "$(tar -tzf "$archive")" == "$BINARY" ]]' in install["run"]
    assert "expected_version=${TAG#iii/v}" in install["run"]
    assert "database -> target" in stack_cache["with"]["workspaces"]


def test_harness_integration_restores_and_saves_final_component_binaries() -> None:
    steps = workflow("_harness-integration.yml")["jobs"]["integration"]["steps"]
    step_names = [step.get("name") for step in steps]
    components = {
        "harness": (
            "Restore harness binaries",
            "Save harness binaries",
            "harness/target/release/harness-integration",
        ),
        "queue": (
            "Restore queue binary",
            "Save queue binary",
            "queue/target/release/queue",
        ),
        "iii-directory": (
            "Restore iii-directory binary",
            "Save iii-directory binary",
            "iii-directory/target/release/iii-directory",
        ),
        "session-manager": (
            "Restore session-manager binary",
            "Save session-manager binary",
            "session-manager/target/release/session-manager",
        ),
        "context-manager": (
            "Restore context-manager binary",
            "Save context-manager binary",
            "context-manager/target/release/context-manager",
        ),
        "state": (
            "Restore state binary",
            "Save state binary",
            "state/target/release/state",
        ),
        "database": (
            "Restore database binary",
            "Save database binary",
            "database/target/release/database",
        ),
        "console": (
            "Restore console binary",
            "Save console binary",
            "console/target/release/console",
        ),
    }

    for component, (restore_name, save_name, binary) in components.items():
        restore = named_step(steps, restore_name)
        save = named_step(steps, save_name)
        assert restore["uses"] == "actions/cache/restore@v4"
        assert binary in restore["with"]["path"]
        assert "integration-component-bin-v1-rust-1.97.1" in restore["with"]["key"]
        assert f"'{component}/**'" in restore["with"]["key"]
        assert save["uses"] == "actions/cache/save@v4"
        assert save["with"]["key"].endswith(".outputs.cache-primary-key }}")
        assert save["if"].startswith("inputs.save-cache &&")
        assert step_names.index(save_name) > step_names.index(
            "Verify integration report links"
        )

    build = named_step(steps, "Build missing integration binaries")["run"]
    run = named_step(steps, "Run integration scenarios")["run"]
    assert "cargo build --locked --release --timings" in build
    assert "steps.harness-bin-cache.outputs.cache-hit" in build
    assert "steps.database-bin-cache.outputs.cache-hit" in build
    assert "integration-run" in run
    assert "integration-test" not in run
    assert all(step.get("name") != "Build Console worker" for step in steps)


def test_makefile_can_build_and_run_the_integration_stack_separately() -> None:
    makefile = (REPOSITORY / "harness" / "Makefile").read_text()
    assert "integration-test: integration-build\n" in makefile
    assert "\t@$(MAKE) --no-print-directory integration-run" in makefile
    assert "integration-build:" in makefile
    assert "integration-run:" in makefile


def test_harness_integration_smokes_the_compose_lifecycle() -> None:
    steps = workflow("_harness-integration.yml")["jobs"]["integration"]["steps"]
    smoke_step = named_step(steps, "Smoke test iii compose")
    fixture_template = (
        REPOSITORY / "harness" / "tests" / "integration" / "compose-smoke.yaml"
    ).read_text()
    fixture = yaml.load(
        fixture_template.replace("@COMPOSE_ENGINE_PORT@", "3210"),
        Loader=yaml.BaseLoader,
    )
    script = (
        REPOSITORY / "harness" / "tests" / "integration" / "compose-smoke.sh"
    ).read_text()

    expected = {
        "queue",
        "iii-directory",
        "session-manager",
        "context-manager",
        "state",
        "database",
    }
    assert smoke_step["env"]["III_BIN"] == "${{ steps.compose-engine.outputs.bin }}"
    assert smoke_step["run"] == "bash harness/tests/integration/compose-smoke.sh"
    assert set(fixture["containers"]) == expected
    assert all(
        container["scripts"]["run"].startswith("@")
        for container in fixture["containers"].values()
    )
    assert 'compose --up --file "$COMPOSE_FILE"' in script
    assert "trigger compose::status" in script
    assert "trigger state::set" in script
    assert "trigger compose::down" in script
    assert 'kill -TERM "$compose_pid"' in script


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
