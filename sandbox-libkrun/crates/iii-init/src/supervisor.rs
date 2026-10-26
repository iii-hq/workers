// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! PID-1 supervision of the user worker process.
//!
//! Spawn the worker via `/bin/sh -c $III_WORKER_CMD` in
//! `$III_WORKER_WORKDIR` and its own process group, forward SIGTERM/SIGINT
//! to it, run the classic `waitpid(-1)` reap loop, and exit with the
//! child's code when it dies.

use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};

use nix::sys::wait::WaitStatus;

use crate::error::InitError;

/// The worker's working directory, set by `__vm-boot`. `/` when unset.
const III_WORKER_WORKDIR_ENV: &str = "III_WORKER_WORKDIR";

/// Virtio-console port name carrying the host↔guest shell-exec channel
/// (`sandbox::exec`, `sandbox::fs::*`). The shell dispatcher runs on a
/// dedicated thread alongside the worker child. If the env var is unset, or
/// the named port can't be found in sysfs, exec is simply unavailable
/// for this VM — the worker child keeps running normally.
const III_SHELL_PORT_ENV: &str = "III_SHELL_PORT";

/// Virtio-console port carrying host source changes. The receiver only writes
/// files; the worker's own watcher remains responsible for process restarts.
const III_SOURCE_SYNC_PORT_ENV: &str = "III_SOURCE_SYNC_PORT";

const III_SOURCE_SYNC_ROOT_ENV: &str = "III_SOURCE_SYNC_ROOT";
const III_SOURCE_SYNC_READY_ENV: &str = "III_SOURCE_SYNC_READY";

/// Stores the current child worker PID for async-signal-safe signal
/// forwarding. 0 means no child has been spawned yet.
static CHILD_PID: AtomicI32 = AtomicI32::new(0);

/// Signal handler that forwards SIGTERM/SIGINT to the current child.
///
/// If no child has been spawned yet (CHILD_PID == 0), exits immediately
/// with code 128 + signal number.
///
/// Only calls async-signal-safe functions: atomic load, libc::kill, libc::_exit.
unsafe extern "C" fn signal_handler(sig: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        unsafe { libc::kill(pid, sig) };
    } else {
        unsafe { libc::_exit(128 + sig) };
    }
}

/// Installs signal handlers for SIGTERM and SIGINT using `libc::sigaction`.
///
/// Handlers are installed with `SA_RESTART` so interrupted syscalls are
/// automatically restarted.
fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = signal_handler as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);

        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }
}

/// Move `pid` into the worker cgroup (best-effort — the cgroup setup is
/// done in `mount.rs` and may not exist on all kernels).
fn attach_to_worker_cgroup(pid: i32) {
    let _ = std::fs::write("/sys/fs/cgroup/worker/cgroup.procs", pid.to_string());
}

/// Entry point called from `main::run` after all boot setup is complete.
///
pub fn exec_worker() -> Result<(), InitError> {
    let cmd = std::env::var("III_WORKER_CMD").map_err(|_| InitError::MissingWorkerCmd)?;

    // Install handlers BEFORE spawning so a signal arriving mid-spawn
    // can still find a live CHILD_PID (racily) or exit cleanly via the
    // _exit fallback.
    install_signal_handlers();
    hold_krun_stdio_ports();

    // Spawn the shell-exec dispatcher thread if wired. Exec sessions are
    // separate children, not substitutes for the worker child. If the
    // named port isn't in
    // sysfs (host forgot `--shell-sock`, or sysfs not mounted), we
    // log and continue without exec; the worker still runs normally.
    maybe_spawn_shell_dispatcher();
    maybe_spawn_source_sync();

    let workdir = std::env::var(III_WORKER_WORKDIR_ENV).unwrap_or_else(|_| "/".to_string());
    run_worker(&cmd, &workdir)
}

fn maybe_spawn_source_sync() {
    let port_name = match std::env::var(III_SOURCE_SYNC_PORT_ENV) {
        Ok(port) => port,
        Err(_) => return,
    };
    let workspace =
        std::env::var(III_SOURCE_SYNC_ROOT_ENV).unwrap_or_else(|_| "/workspace".to_string());
    let ready = std::env::var(III_SOURCE_SYNC_READY_ENV)
        .unwrap_or_else(|_| "/run/iii/source-ready".to_string());

    match iii_supervisor::virtio::find_virtio_port_by_name(&port_name) {
        Some(port_path) => {
            std::thread::Builder::new()
                .name("iii-init-source-sync".to_string())
                .spawn(move || {
                    if let Err(error) = crate::source_sync::run(
                        &port_path,
                        Path::new(&workspace),
                        Path::new(&ready),
                    ) {
                        eprintln!("iii-init: source sync error: {error}");
                    }
                })
                .expect("spawn source sync thread");
        }
        None => {
            eprintln!(
                "iii-init: warning: III_SOURCE_SYNC_PORT={port_name} but no matching port \
                 in /sys/class/virtio-ports. Host source changes will not be forwarded."
            );
        }
    }
}

/// Look up `III_SHELL_PORT`, resolve the named virtio-console port via
/// sysfs, and spawn the dispatcher thread on it. Silently returns if
/// the env var is unset (feature not enabled for this VM) and logs a
/// warning if it's set but the port can't be found.
///
/// The spawned thread runs for the lifetime of the VM. It does not
/// block or join back with PID 1 — its death (port closed, read
/// error) simply disables future exec sessions; the worker and any
/// in-flight exec children keep running.
fn maybe_spawn_shell_dispatcher() {
    let port_name = match std::env::var(III_SHELL_PORT_ENV) {
        Ok(p) => p,
        Err(_) => return,
    };
    match iii_supervisor::virtio::find_virtio_port_by_name(&port_name) {
        Some(port_path) => {
            std::thread::Builder::new()
                .name("iii-init-shell".to_string())
                .spawn(move || {
                    if let Err(e) = crate::shell_dispatcher::run(&port_path) {
                        eprintln!("iii-init: shell dispatcher error: {e}");
                    }
                })
                .expect("spawn shell dispatcher thread");
        }
        None => {
            eprintln!(
                "iii-init: warning: III_SHELL_PORT={port_name} but no matching port \
                 in /sys/class/virtio-ports. sandbox exec disabled for this VM."
            );
        }
    }
}

/// The console ports libkrun adds for `__vm-boot`'s non-TTY stdio. Nothing in
/// the guest uses them.
const KRUN_STDIO_PORTS: [&str; 3] = ["krun-stdin", "krun-stdout", "krun-stderr"];

/// Holds libkrun's stdio ports open for PID 1's life, as `worker_env` holds
/// `iii.env`: a guest process that opens one again then gets EBUSY, where a
/// second open of the port made the host's console device kill the VM.
/// shortcut: a port the kernel names only after this pass stays unheld; poll
/// for the names if that is ever seen.
fn hold_krun_stdio_ports() {
    for name in KRUN_STDIO_PORTS {
        if let Some(path) = iii_supervisor::virtio::find_virtio_port_by_name(name)
            && let Err(e) = hold_port(&path)
        {
            eprintln!("iii-init: warning: cannot hold {}: {e}", path.display());
        }
    }
}

/// Opens `path` and never closes it. Close-on-exec, so no worker inherits it.
fn hold_port(path: &Path) -> std::io::Result<()> {
    std::mem::forget(std::fs::File::open(path)?);
    Ok(())
}

/// Spawn the worker, reap zombies, exit with the worker's code.
fn run_worker(cmd: &str, workdir: &str) -> Result<(), InitError> {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(workdir)
        .stdin(Stdio::null())
        // Its own process group, apart from PID 1 and the exec sessions.
        .process_group(0)
        .spawn()
        .map_err(InitError::SpawnWorker)?;

    let child_pid = child.id() as i32;
    CHILD_PID.store(child_pid, Ordering::SeqCst);

    attach_to_worker_cgroup(child_pid);

    // PID 1 supervisor loop: wait for children, reap orphans (INIT-07).
    // Before applying our own termination logic, consult the shell
    // dispatcher's exit registry — exits belonging to exec children
    // must be forwarded to the dispatcher's waiter thread, not silently
    // discarded as "orphans".
    let status = loop {
        match crate::child_exits::wait_any() {
            Ok(WaitStatus::Exited(pid, code)) => {
                if crate::child_exits::dispatch_exit(pid.as_raw(), code) {
                    continue;
                }
                if pid.as_raw() == child_pid {
                    break code;
                }
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                if crate::child_exits::dispatch_exit(pid.as_raw(), 128 + sig as i32) {
                    continue;
                }
                if pid.as_raw() == child_pid {
                    break 128 + sig as i32;
                }
            }
            Ok(_) => continue,                  // stop/continue/etc, keep waiting
            Err(nix::Error::ECHILD) => break 0, // no more children
            Err(_) => break 1,                  // unexpected error
        }
    };

    sync_and_exit(status);
}

/// Flush filesystems, then exit PID 1 (which tears the VM down).
///
/// On worker exit we go straight to teardown: libkrun calls `_exit()`, which
/// skips every Drop-based flush, and nothing else syncs the writable overlay
/// upper (ext4 on /dev/vdb). Installed deps + the `/var/.iii-prepared` marker
/// would otherwise depend on ext4's ~5s background commit happening to fire
/// before teardown, so a fast-exiting or crash-looping worker loses them and
/// re-runs install next boot. One `sync()` here makes every clean PID-1 exit
/// durable; it runs once as the VM is dying, so the cost is irrelevant.
/// ponytail: covers clean + SIGTERM/SIGINT exits (those funnel back through the
/// reap loop); a host `kill -9`/OOM mid-window runs no code — mount the upper
/// with `commit=1` if that residual window ever matters.
pub fn sync_and_exit(status: i32) -> ! {
    nix::unistd::sync();
    // libkrun takes the VM's exit code from this ioctl on a virtiofs mount
    // (VIRTIO_IOC_EXIT_CODE_REQ, `_IO('v', 2)`); without it `__vm-boot` exits 0
    // whatever the worker returned, and a crashed worker read as a clean exit.
    if let Some(root) = VIRTIOFS_ROOT.get() {
        unsafe { libc::ioctl(root.as_raw_fd(), 0x7602 as _, status) };
    }
    std::process::exit(status);
}

/// The virtiofs root, opened before the root pivot detaches it, for the
/// exit-code ioctl in [`sync_and_exit`]. Close-on-exec, so no worker inherits it.
static VIRTIOFS_ROOT: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

/// Keep the virtiofs root open; call it before the root pivot.
pub fn hold_virtiofs_root() {
    match std::fs::File::open("/") {
        Ok(root) => {
            let _ = VIRTIOFS_ROOT.set(root);
        }
        Err(e) => eprintln!("iii-init: warning: cannot open / for the exit code: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_worker_cmd() {
        // Ensure III_WORKER_CMD is not set in this test's environment.
        // SAFETY: remove_var is unsafe in edition 2024 because it is inherently
        // racy in multi-threaded programs. This is acceptable in a test where we
        // control the environment.
        unsafe { std::env::remove_var("III_WORKER_CMD") };
        let result = exec_worker();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, InitError::MissingWorkerCmd),
            "expected MissingWorkerCmd, got: {err}"
        );
    }

    /// A held port stays open: a FIFO keeps accepting a non-blocking writer
    /// only while some reader has it open.
    #[test]
    fn a_held_port_stays_open() {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("krun-stdout");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
        let writer = || {
            std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo)
        };
        assert!(writer().is_err(), "no reader yet, so ENXIO");
        // A plain open of a FIFO waits for a writer; the port does not.
        let opener = std::thread::spawn({
            let fifo = fifo.clone();
            move || hold_port(&fifo)
        });
        let keep_open = loop {
            if let Ok(w) = writer() {
                break w;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        opener.join().unwrap().unwrap();
        drop(keep_open);
        assert!(writer().is_ok(), "the held port was closed");
    }

    #[test]
    fn test_signal_handler_signature() {
        // Compile-time check that the signal handler has the correct
        // extern "C" fn(c_int) signature required by libc::sigaction.
        let _: unsafe extern "C" fn(libc::c_int) = signal_handler;
    }
}
