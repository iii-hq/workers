//! CLI smoke tests for the `iii-mcp` binary.
//!
//! `iii-mcp` exposes no `--manifest` subcommand; instead we exercise the
//! flags that resolve without an engine connection (`--version` and
//! `--help`). This guards the binary entry-point: clap's derive expansion,
//! the package version baked in by Cargo, and the help text that ops uses
//! to discover the available transports.

use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_iii-mcp")
}

#[test]
fn version_flag_reports_cargo_pkg_version() {
    let output = Command::new(binary())
        .arg("--version")
        .output()
        .expect("spawn iii-mcp --version");

    assert!(
        output.status.success(),
        "iii-mcp --version exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("--version stdout is utf-8");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "expected --version output to contain {}, got: {stdout:?}",
        env!("CARGO_PKG_VERSION"),
    );
}

#[test]
fn help_flag_advertises_known_transport_flags() {
    let output = Command::new(binary())
        .arg("--help")
        .output()
        .expect("spawn iii-mcp --help");

    assert!(
        output.status.success(),
        "iii-mcp --help exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let help = String::from_utf8(output.stdout).expect("--help stdout is utf-8");
    for flag in [
        "--engine-url",
        "--no-stdio",
        "--no-builtins",
        "--http-builtins",
        "--rbac-tag",
    ] {
        assert!(help.contains(flag), "--help should mention {flag}: {help}");
    }
}
