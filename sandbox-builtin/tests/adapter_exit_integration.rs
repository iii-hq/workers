// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! How the adapter exits: a failed config fetch fails closed, and SIGTERM
//! (what Compose sends) is a graceful exit.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

const EXIT_DEADLINE: Duration = Duration::from_secs(15);

struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_log_line(logfile: &Path, needle: &str, deadline: Duration) -> (bool, String) {
    let end = Instant::now() + deadline;
    loop {
        let contents = std::fs::read_to_string(logfile).unwrap_or_default();
        if contents.contains(needle) {
            return (true, contents);
        }
        if Instant::now() >= end {
            return (false, contents);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_exit(
    child: &mut KillOnDrop,
    deadline: Duration,
    logfile: &Path,
) -> std::process::ExitStatus {
    let end = Instant::now() + deadline;
    loop {
        if let Some(status) = child.0.try_wait().expect("try_wait") {
            return status;
        }
        if Instant::now() >= end {
            let log = std::fs::read_to_string(logfile).unwrap_or_default();
            panic!("sandbox-builtin did not exit within {deadline:?}; log:\n{log}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The bare binary against a dead engine URL: its config fetch keeps retrying
/// (three 5 s attempts).
fn adapter_command(tmp: &Path, logfile: &Path) -> Command {
    let log = std::fs::File::create(logfile).unwrap();
    let log_err = log.try_clone().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sandbox-builtin"));
    cmd.env("III_URL", "ws://127.0.0.1:1")
        .current_dir(tmp)
        .env("RUST_LOG", "info")
        .env("HOME", tmp)
        .env_remove("III_LIFELINE_FD")
        .env_remove("III_LIFELINE_SPAWNER_PID")
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    cmd
}

#[test]
fn config_fetch_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    let logfile = tmp.path().join("sandbox-builtin.out");

    let mut adapter = KillOnDrop(
        adapter_command(tmp.path(), &logfile)
            .spawn()
            .expect("spawn sandbox-builtin"),
    );

    let status = wait_for_exit(&mut adapter, Duration::from_secs(60), &logfile);
    let log = std::fs::read_to_string(&logfile).unwrap_or_default();
    assert_eq!(
        status.code(),
        Some(2),
        "a failed config fetch must fail closed; log:\n{log}"
    );
    assert!(log.contains("aborting (fail-closed)"), "log:\n{log}");
}

#[test]
fn sigterm_exits_gracefully() {
    let tmp = tempfile::tempdir().unwrap();
    let logfile = tmp.path().join("sandbox-builtin.out");
    let mut adapter = KillOnDrop(
        adapter_command(tmp.path(), &logfile)
            .spawn()
            .expect("spawn sandbox-builtin"),
    );

    let (armed, log) =
        wait_for_log_line(&logfile, "shutdown signals armed", Duration::from_secs(10));
    assert!(
        armed,
        "sandbox-builtin never armed its shutdown signals; log:\n{log}"
    );

    kill(Pid::from_raw(adapter.0.id() as i32), Signal::SIGTERM).expect("send SIGTERM");
    let status = wait_for_exit(&mut adapter, EXIT_DEADLINE, &logfile);
    let log = std::fs::read_to_string(&logfile).unwrap_or_default();
    assert_eq!(
        status.code(),
        Some(0),
        "SIGTERM exit must be graceful; log:\n{log}"
    );
    assert!(log.contains("sigterm"), "log:\n{log}");
}
