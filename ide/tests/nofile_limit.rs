//! The worker raises its own open-file limit at startup, and every command it
//! runs inherits it. A terminal on macOS starts processes with a soft limit of
//! 256, which a Rust build or test run exhausts.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const LOW: libc::rlim_t = 256;

#[test]
fn the_worker_raises_a_low_open_file_limit_at_startup() {
    let mut inherited = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit only writes the struct it is given.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut inherited) },
        0
    );
    if inherited.rlim_max <= LOW {
        eprintln!(
            "[skip] a hard open-file limit of {} leaves nothing to raise",
            inherited.rlim_max
        );
        return;
    }

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ide"));
    cmd.args(["--url", "ws://127.0.0.1:9"])
        .current_dir(std::env::temp_dir())
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: the hook runs in the forked child before exec and only makes
    // async-signal-safe rlimit syscalls on a local struct.
    unsafe {
        cmd.pre_exec(|| {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            limit.rlim_cur = LOW;
            if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("start the worker");
    let stdout = child.stdout.take().expect("piped stdout");
    let (found, wait) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains("raised open file limit") {
                let _ = found.send(line);
                return;
            }
        }
    });
    let line = wait.recv_timeout(Duration::from_secs(20));
    let _ = child.kill();
    let _ = child.wait();
    let line = line.expect("the worker logs the raise before it connects");
    assert!(line.contains(&format!("from={LOW}")), "{line}");
}
