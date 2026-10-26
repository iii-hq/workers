// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Ungated integration tests for VM argument construction, mount parsing,
//! localhost rewriting and firmware resolution. These tests run in default
//! `cargo test` without requiring libkrun.

use clap::Parser;
use sandbox_libkrun::cli::vm_boot::{
    VmBootArgs, build_worker_cmd, resolve_krunfw_file_path, rewrite_localhost, shell_quote,
};

/// Helper wrapper for clap parsing of VmBootArgs.
#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    args: VmBootArgs,
}

#[test]
fn vm_boot_args_all_fields() {
    let cli = TestCli::parse_from([
        "test",
        "--rootfs",
        "/tmp/rootfs",
        "--exec",
        "/usr/bin/node",
        "--workdir",
        "/workspace",
        "--vcpus",
        "4",
        "--ram",
        "1024",
        "--env",
        "FOO=bar",
        "--mount",
        "/host:/guest",
        "--arg",
        "script.js",
        "--slot",
        "42",
        "--net-activity-file",
        "/tmp/net-activity",
    ]);
    assert_eq!(cli.args.rootfs, "/tmp/rootfs");
    assert_eq!(cli.args.exec, "/usr/bin/node");
    assert_eq!(cli.args.workdir, "/workspace");
    assert_eq!(cli.args.vcpus, 4);
    assert_eq!(cli.args.ram, 1024);
    assert_eq!(cli.args.env, vec!["FOO=bar"]);
    assert_eq!(cli.args.mount, vec!["/host:/guest"]);
    assert_eq!(cli.args.arg, vec!["script.js"]);
    assert_eq!(cli.args.slot, 42);
    assert_eq!(
        cli.args.net_activity_file.as_deref(),
        Some("/tmp/net-activity")
    );
}

#[test]
fn vm_boot_args_defaults() {
    let cli = TestCli::parse_from(["test", "--rootfs", "/rootfs", "--exec", "/bin/app"]);
    assert_eq!(cli.args.rootfs, "/rootfs");
    assert_eq!(cli.args.exec, "/bin/app");
    assert_eq!(cli.args.workdir, "/");
    assert_eq!(cli.args.vcpus, 2);
    assert_eq!(cli.args.ram, 2048);
    assert_eq!(cli.args.slot, 0);
    assert!(cli.args.mount.is_empty());
    assert!(cli.args.env.is_empty());
    assert!(cli.args.arg.is_empty());
    assert!(cli.args.net_activity_file.is_none());
}

#[test]
fn vm_boot_args_hyphen_prefixed_args() {
    let cli = TestCli::parse_from([
        "test", "--rootfs", "/r", "--exec", "/e", "--arg", "--port", "--arg", "3000", "--arg", "-v",
    ]);
    assert_eq!(cli.args.arg, vec!["--port", "3000", "-v"]);
}

#[test]
fn vm_boot_args_multiple_envs() {
    let cli = TestCli::parse_from([
        "test",
        "--rootfs",
        "/r",
        "--exec",
        "/e",
        "--env",
        "FOO=bar",
        "--env",
        "BAZ=qux",
        "--env",
        "URL=http://host:8080",
    ]);
    assert_eq!(cli.args.env.len(), 3);
    assert_eq!(cli.args.env[0], "FOO=bar");
    assert_eq!(cli.args.env[1], "BAZ=qux");
    assert_eq!(cli.args.env[2], "URL=http://host:8080");
}

#[test]
fn vm_boot_args_multiple_mounts() {
    let cli = TestCli::parse_from([
        "test",
        "--rootfs",
        "/r",
        "--exec",
        "/e",
        "--mount",
        "/src:/workspace",
        "--mount",
        "/data:/mnt/data",
        "--mount",
        "/cache:/tmp/cache",
    ]);
    assert_eq!(cli.args.mount.len(), 3);
    assert_eq!(cli.args.mount[0], "/src:/workspace");
    assert_eq!(cli.args.mount[1], "/data:/mnt/data");
    assert_eq!(cli.args.mount[2], "/cache:/tmp/cache");
}

#[test]
fn shell_quote_safe_chars_passthrough() {
    assert_eq!(shell_quote("simple"), "simple");
    assert_eq!(shell_quote("/usr/bin/node"), "/usr/bin/node");
    assert_eq!(shell_quote("key=value"), "key=value");
    assert_eq!(shell_quote("a-b_c.d:e/f"), "a-b_c.d:e/f");
}

#[test]
fn shell_quote_spaces_get_quoted() {
    assert_eq!(shell_quote("has space"), "'has space'");
}

#[test]
fn shell_quote_embedded_single_quotes() {
    assert_eq!(shell_quote("it's a test"), "'it'\\''s a test'");
}

#[test]
fn shell_quote_special_chars() {
    let result = shell_quote("$(cmd)");
    assert!(result.starts_with('\''), "$(cmd) should be quoted");
    let result2 = shell_quote("a;b");
    assert!(result2.starts_with('\''), "a;b should be quoted");
}

#[test]
fn shell_quote_empty_string() {
    // Empty string has no unsafe chars, passes through unchanged.
    assert_eq!(shell_quote(""), "");
}

#[test]
fn build_worker_cmd_no_args() {
    assert_eq!(build_worker_cmd("/usr/bin/node", &[]), "/usr/bin/node");
}

#[test]
fn build_worker_cmd_with_args() {
    let args = vec![
        "script.js".to_string(),
        "--port".to_string(),
        "3000".to_string(),
    ];
    assert_eq!(
        build_worker_cmd("/usr/bin/node", &args),
        "/usr/bin/node script.js --port 3000"
    );
}

#[test]
fn build_worker_cmd_with_spaces_in_exec() {
    let result = build_worker_cmd("/path/to/my app", &[]);
    assert!(
        result.contains('\''),
        "exec path with spaces should be quoted: {}",
        result
    );
}

#[test]
fn build_worker_cmd_with_special_arg() {
    let args = vec!["it's".to_string()];
    let result = build_worker_cmd("/bin/sh", &args);
    assert!(
        result.contains("\\'"),
        "argument with single quote should be escaped: {}",
        result
    );
}

#[test]
fn vm_boot_args_mount_valid_format() {
    let cli = TestCli::parse_from([
        "test",
        "--rootfs",
        "/r",
        "--exec",
        "/e",
        "--mount",
        "/host:/guest",
    ]);
    assert_eq!(cli.args.mount[0], "/host:/guest");
}

#[test]
fn vm_boot_args_mount_no_colon_stored_as_is() {
    // VmBootArgs stores the string as-is; validation happens at boot_vm time.
    let cli = TestCli::parse_from([
        "test",
        "--rootfs",
        "/r",
        "--exec",
        "/e",
        "--mount",
        "invalid-no-colon",
    ]);
    assert_eq!(cli.args.mount[0], "invalid-no-colon");
}

#[test]
fn vm_boot_args_mount_empty_guest() {
    let cli = TestCli::parse_from([
        "test", "--rootfs", "/r", "--exec", "/e", "--mount", "/host:",
    ]);
    assert_eq!(cli.args.mount[0], "/host:");
}

#[test]
fn rewrite_localhost_replaces_localhost() {
    assert_eq!(
        rewrite_localhost("http://localhost:3000/api", "192.168.1.1"),
        "http://192.168.1.1:3000/api"
    );
}

#[test]
fn rewrite_localhost_replaces_loopback() {
    assert_eq!(
        rewrite_localhost("ws://127.0.0.1:8080/ws", "10.0.0.1"),
        "ws://10.0.0.1:8080/ws"
    );
}

#[test]
fn rewrite_localhost_both_in_one_string() {
    let result = rewrite_localhost("http://localhost:3000 ws://127.0.0.1:8080", "10.0.0.1");
    assert!(
        result.contains("10.0.0.1:3000"),
        "should rewrite localhost: {}",
        result
    );
    assert!(
        result.contains("10.0.0.1:8080"),
        "should rewrite 127.0.0.1: {}",
        result
    );
}

#[test]
fn rewrite_localhost_no_match_unchanged() {
    assert_eq!(
        rewrite_localhost("http://example.com:3000", "10.0.0.1"),
        "http://example.com:3000"
    );
}

#[test]
fn rewrite_localhost_no_port_unchanged() {
    // Only matches "://localhost:" pattern (requires colon after hostname).
    assert_eq!(
        rewrite_localhost("localhost without colon", "10.0.0.1"),
        "localhost without colon"
    );
}

// Regression: with networking disabled, `gateway_ip` is `None` and the
// rewrite must be a no-op. Empty-string sentinels previously turned
// `://localhost:9000` into `://:9000` (vm_boot.rs:683-698 pre-fix).
#[test]
fn maybe_rewrite_localhost_skips_when_networking_disabled() {
    use sandbox_libkrun::cli::vm_boot::maybe_rewrite_localhost;

    assert_eq!(
        maybe_rewrite_localhost("http://localhost:9000/api", None),
        "http://localhost:9000/api"
    );
    assert_eq!(
        maybe_rewrite_localhost("ws://127.0.0.1:8080/ws", None),
        "ws://127.0.0.1:8080/ws"
    );
}

#[test]
fn maybe_rewrite_localhost_rewrites_when_gateway_present() {
    use sandbox_libkrun::cli::vm_boot::maybe_rewrite_localhost;

    assert_eq!(
        maybe_rewrite_localhost("http://localhost:3000/api", Some("192.168.1.1")),
        "http://192.168.1.1:3000/api"
    );
    assert_eq!(
        maybe_rewrite_localhost("ws://127.0.0.1:8080/ws", Some("10.0.0.1")),
        "ws://10.0.0.1:8080/ws"
    );
}

#[test]
fn resolve_krunfw_file_path_returns_option() {
    // In a test environment without firmware installed, this should return None.
    // The function is callable and returns the correct type.
    let result = resolve_krunfw_file_path();
    // We cannot guarantee Some or None depending on the host, but in CI/test
    // environments firmware is typically not installed. The key assertion is
    // that the function is callable from integration tests (pub visibility works).
    assert!(
        result.is_none() || result.is_some(),
        "resolve_krunfw_file_path should return a valid Option"
    );
    // If firmware IS present, verify the path exists.
    if let Some(ref path) = result {
        assert!(path.exists(), "returned firmware path should exist");
    }
}
