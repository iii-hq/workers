"""Unused Rust dependencies are reported as CI warnings, never as failures."""

from __future__ import annotations

import os
from pathlib import Path
import shlex
import subprocess

import yaml


GITHUB = Path(__file__).parents[2]
SCRIPT = GITHUB / "scripts" / "cargo_shear_warnings.sh"
INSTALL_ACTION = "taiki-e/install-action@82cd3e7658a6f96c86c0234aeeda1748937cb0a1"


def run_with_fake_cargo(
    tmp_path: Path, output: str, exit_code: int
) -> subprocess.CompletedProcess[str]:
    """Run the script with a `cargo` that prints `output` and exits `exit_code`."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    cargo = bin_dir / "cargo"
    cargo.write_text(
        f"#!/usr/bin/env bash\nprintf '%s\\n' {shlex.quote(output)}\nexit {exit_code}\n"
    )
    cargo.chmod(0o755)
    return subprocess.run(
        ["bash", str(SCRIPT), "crates/judge-provider/"],
        cwd=tmp_path,
        env={**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}"},
        capture_output=True,
        text=True,
        check=False,
    )


def test_findings_become_warnings_on_the_repository_path(tmp_path: Path) -> None:
    result = run_with_fake_cargo(
        tmp_path,
        "::error file=Cargo.toml,line=28,col=1,title=shear/unused_dependency::"
        "unused dependency `tempfile` (remove this dependency)",
        1,
    )

    assert result.returncode == 0
    assert "::error" not in result.stdout
    assert (
        "::warning file=crates/judge-provider/Cargo.toml,line=28,col=1,"
        "title=shear/unused_dependency::unused dependency `tempfile`"
    ) in result.stdout
    assert "cargo shear --fix" in result.stdout


def test_a_check_that_cannot_run_warns_instead_of_failing(tmp_path: Path) -> None:
    result = run_with_fake_cargo(tmp_path, "error: no such command: `shear`", 101)

    assert result.returncode == 0
    assert "error: no such command: `shear`" in result.stdout
    assert "::warning title=crates/judge-provider: unused dependencies::" in result.stdout
    assert "could not run in crates/judge-provider (exit 101)" in result.stdout


def test_a_clean_crate_says_so(tmp_path: Path) -> None:
    result = run_with_fake_cargo(tmp_path, "", 0)

    assert result.returncode == 0
    assert "::warning" not in result.stdout
    assert "crates/judge-provider: no unused dependencies." in result.stdout


def test_every_rust_lint_job_warns_about_unused_dependencies() -> None:
    ci = yaml.load((GITHUB / "workflows" / "ci.yml").read_text(), Loader=yaml.BaseLoader)
    for job, variable, value in (
        ("rust", "WORKER", "${{ matrix.worker }}"),
        ("crates", "CRATE", "crates/${{ matrix.crate }}"),
    ):
        steps = {step.get("name"): step for step in ci["jobs"][job]["steps"]}
        install = steps["Install cargo-shear"]
        assert install["uses"] == INSTALL_ACTION
        assert install["with"]["tool"] == "cargo-shear@1.13.4"
        # A failed install must not fail the job; the check then warns.
        assert install["continue-on-error"] == "true"

        check = steps["Check unused dependencies (warning only)"]
        assert check["env"] == {variable: value}
        assert ".github/scripts/cargo_shear_warnings.sh" in check["run"]
        assert f'"${variable}"' in check["run"]
