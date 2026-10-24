// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `__vm-boot --env-stdin` as a real subprocess (MOT-5492). No hypervisor
//! needed: `run()` reads the stdin frame before anything else, and a
//! nonexistent rootfs makes it exit right after.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use iii_supervisor::env_channel::encode_frame;

fn spawn_vm_boot() -> Child {
    Command::new(env!("CARGO_BIN_EXE_sandbox-builtin"))
        .args([
            "__vm-boot",
            "--rootfs",
            "/nonexistent/mot-5492",
            "--exec",
            "/bin/true",
            "--env-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sandbox-builtin __vm-boot")
}

/// Exit code and stderr, failing instead of hanging if the child never exits.
fn finish(mut child: Child) -> (i32, String) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "__vm-boot did not exit");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (status.code().unwrap_or(-1), stderr)
}

fn vm_boot(stdin: &[u8]) -> (i32, String) {
    let mut child = spawn_vm_boot();
    // Blocks if the child stops reading early; a hang here is the bug.
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    finish(child)
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn a_valid_frame_is_read_before_the_rootfs_check_and_never_echoed() {
    let frame = encode_frame(&pairs(&[("API_KEY", "sk-secret"), ("GREETING", "Olá")])).unwrap();
    let (code, stderr) = vm_boot(&frame);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("rootfs path does not exist"), "{stderr}");
    assert!(!stderr.contains("sk-secret"), "{stderr}");
}

#[test]
fn a_frame_larger_than_the_pipe_buffer_never_blocks_the_launcher() {
    let big = "x".repeat(120 * 1024);
    let (code, stderr) = vm_boot(&encode_frame(&pairs(&[("BIG", &big)])).unwrap());
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("rootfs path does not exist"), "{stderr}");
}

/// A copy of the pipe's write end leaked into another child holds off EOF;
/// the length prefix means `__vm-boot` never waits for it.
#[test]
fn a_frame_on_a_pipe_left_open_is_read_without_waiting_for_eof() {
    let mut child = spawn_vm_boot();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(&encode_frame(&pairs(&[("A", "1")])).unwrap())
        .unwrap();
    let (code, stderr) = finish(child);
    drop(stdin);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("rootfs path does not exist"), "{stderr}");
}

#[test]
fn a_missing_or_rejected_frame_fails_naming_only_the_key() {
    let (code, stderr) = vm_boot(b"");
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("--env-stdin"), "{stderr}");

    // encode_frame refuses a NUL, so frame it by hand.
    let payload = br#"[["API_KEY","sk-secret\u0000tail"]]"#;
    let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(payload);
    let (code, stderr) = vm_boot(&frame);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("API_KEY"), "{stderr}");
    assert!(!stderr.contains("sk-secret"), "{stderr}");
}
