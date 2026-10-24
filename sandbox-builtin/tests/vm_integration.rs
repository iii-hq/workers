// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Feature-gated VM integration tests.
//! Run with: cargo test --test vm_integration --features integration-vm
//!
//! These tests verify `__vm-boot` argument parsing, localhost rewriting of the
//! worker env, and actual `__vm-boot` subprocess argument passing.

#![cfg(feature = "integration-vm")]

use clap::Parser;
use sandbox_builtin::cli::vm_boot::{VmBootArgs, rewrite_localhost};

/// Helper wrapper for clap parsing of VmBootArgs.
#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    args: VmBootArgs,
}

// ===========================================================================
// Group 1: Feature gate verification (sanity check)
// ===========================================================================

#[test]
fn vm_feature_gate_compiles() {
    const {
        assert!(
            cfg!(feature = "integration-vm"),
            "integration-vm feature should be active when this file compiles"
        )
    };
}

// ===========================================================================
// Group 2: VmBootArgs parsing and env rewriting
// ===========================================================================

/// Parse with vcpus at u8 boundary and beyond. VmBootArgs accepts u32;
/// the u8 check happens at boot_vm runtime, not at parse time.
#[test]
fn vm_boot_args_max_vcpus_at_u8_boundary() {
    // u8::MAX (255) should parse fine
    let cli = TestCli::parse_from(["test", "--rootfs", "/r", "--exec", "/e", "--vcpus", "255"]);
    assert_eq!(cli.args.vcpus, 255);

    // 256 should also parse (VmBootArgs field is u32)
    let cli = TestCli::parse_from(["test", "--rootfs", "/r", "--exec", "/e", "--vcpus", "256"]);
    assert_eq!(cli.args.vcpus, 256);

    // Large value within u32 range
    let cli = TestCli::parse_from(["test", "--rootfs", "/r", "--exec", "/e", "--vcpus", "65535"]);
    assert_eq!(cli.args.vcpus, 65535);
}

/// Verify that rewrite_localhost correctly transforms environment variable
/// values containing localhost URLs, as happens during VM boot.
#[test]
fn env_values_with_localhost_get_rewritten() {
    let env_vars: Vec<(&str, &str)> = vec![
        ("III_ENGINE_URL", "ws://localhost:49134"),
        ("DATABASE_URL", "postgres://127.0.0.1:5432/db"),
        ("SIMPLE_VAR", "no-url-here"),
    ];
    let gateway_ip = "192.168.64.2";

    let rewritten: Vec<(String, String)> = env_vars
        .iter()
        .map(|(k, v)| (k.to_string(), rewrite_localhost(v, gateway_ip)))
        .collect();

    // III_ENGINE_URL: localhost -> gateway_ip
    let iii_url = &rewritten[0].1;
    assert!(
        iii_url.contains("192.168.64.2:49134"),
        "III_ENGINE_URL should have rewritten localhost: {}",
        iii_url
    );
    assert!(
        !iii_url.contains("localhost"),
        "III_ENGINE_URL should not contain localhost after rewrite: {}",
        iii_url
    );

    // DATABASE_URL: 127.0.0.1 -> gateway_ip
    let db_url = &rewritten[1].1;
    assert!(
        db_url.contains("192.168.64.2:5432"),
        "DATABASE_URL should have rewritten 127.0.0.1: {}",
        db_url
    );
    assert!(
        !db_url.contains("127.0.0.1"),
        "DATABASE_URL should not contain 127.0.0.1 after rewrite: {}",
        db_url
    );

    // SIMPLE_VAR: no URL, unchanged
    assert_eq!(
        rewritten[2].1, "no-url-here",
        "SIMPLE_VAR should be unchanged"
    );
}

// ===========================================================================
// Group 3: Actual subprocess argument verification
// ===========================================================================

/// Spawn the real sandbox-builtin __vm-boot subprocess with a valid temp directory
/// as rootfs. The subprocess will fail because the rootfs has no init binary
/// and no firmware, but its stderr output will contain the rootfs path --
/// proving it received and used the --rootfs argument correctly.
// An embedded init skips the "No init binary available" failure and boots a real VM.
#[cfg(not(feature = "embed-init"))]
#[test]
fn vm_boot_subprocess_receives_correct_rootfs_argument() {
    let temp_dir = tempfile::TempDir::new().expect("failed to create temp dir");
    let temp_dir_path = temp_dir.path().to_string_lossy().to_string();

    let binary = env!("CARGO_BIN_EXE_sandbox-builtin");
    let output = std::process::Command::new(binary)
        .arg("__vm-boot")
        .arg("--rootfs")
        .arg(&temp_dir_path)
        .arg("--exec")
        .arg("/usr/bin/node")
        .arg("--workdir")
        .arg("/workspace")
        .arg("--vcpus")
        .arg("2")
        .arg("--ram")
        .arg("2048")
        .arg("--env")
        .arg("III_ENGINE_URL=ws://localhost:49134")
        .arg("--env")
        .arg("III_WORKER_NAME=test-subprocess")
        .arg("--arg")
        .arg("--url")
        .arg("--arg")
        .arg("ws://localhost:49134")
        .output()
        .expect("failed to execute sandbox-builtin __vm-boot subprocess");

    // The subprocess should exit with non-zero status (no init binary)
    assert!(
        !output.status.success(),
        "subprocess should fail (no init binary in rootfs)"
    );

    let stderr_str = String::from_utf8_lossy(&output.stderr);

    // The key assertion: stderr contains the rootfs path, proving the
    // subprocess received the --rootfs argument and attempted to use it.
    // The error comes from vm_boot::run() or boot_vm() which includes
    // the rootfs path in its error message.
    assert!(
        stderr_str.contains(&temp_dir_path),
        "stderr should contain the rootfs path '{}' proving argument receipt.\nActual stderr: {}",
        temp_dir_path,
        stderr_str
    );

    // Verify the error message is from the expected validation path
    let is_known_error = stderr_str.contains("No init binary available")
        || stderr_str.contains("VM execution failed")
        || stderr_str.contains("PassthroughFs failed");
    assert!(
        is_known_error,
        "stderr should contain a known VM boot error message.\nActual stderr: {}",
        stderr_str
    );
}

/// Spawn the real sandbox-builtin __vm-boot subprocess with a nonexistent rootfs
/// path. The subprocess should fail immediately with the early validation
/// error in vm_boot::run() and report the path in stderr.
#[test]
fn vm_boot_subprocess_rejects_nonexistent_rootfs() {
    let binary = env!("CARGO_BIN_EXE_sandbox-builtin");
    let nonexistent_path = "/nonexistent/path/that/does/not/exist";

    let output = std::process::Command::new(binary)
        .arg("__vm-boot")
        .arg("--rootfs")
        .arg(nonexistent_path)
        .arg("--exec")
        .arg("/bin/sh")
        .arg("--vcpus")
        .arg("1")
        .arg("--ram")
        .arg("512")
        .output()
        .expect("failed to execute sandbox-builtin __vm-boot subprocess");

    // The subprocess should exit with non-zero status
    assert!(
        !output.status.success(),
        "subprocess should fail for nonexistent rootfs"
    );

    let stderr_str = String::from_utf8_lossy(&output.stderr);

    // Verify the early validation message from run() line 143-145
    assert!(
        stderr_str.contains("rootfs path does not exist"),
        "stderr should contain 'rootfs path does not exist'.\nActual stderr: {}",
        stderr_str
    );

    // Verify the nonexistent path appears in stderr
    assert!(
        stderr_str.contains(nonexistent_path),
        "stderr should contain the nonexistent path '{}'.\nActual stderr: {}",
        nonexistent_path,
        stderr_str
    );
}
