// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! The adapter's shutdown signals and the adapter-to-VM lifeline.
//!
//! The adapter stops on SIGINT, SIGTERM or SIGHUP
//! ([`wait_for_shutdown_signal`]); Compose stops it with SIGTERM. Every
//! `__vm-boot` it spawns gets a lifeline pipe ([`attach_lifeline`]): the
//! adapter holds the write end, so when it dies, however abruptly, the VM's
//! read sees EOF and the VM shuts down. A pid poll on the adapter backstops
//! the one way the lifeline can fail (see [`attach_lifeline`]).
//!
//! Unix-only: on non-Unix targets only Ctrl-C is watched and there is no
//! lifeline.

#[cfg(unix)]
use std::time::Duration;

/// Env var naming the file-descriptor number of an inherited LIFELINE PIPE
/// read end. The spawner holds the write end and never writes; when the
/// spawner dies — for ANY reason, SIGKILL included — the kernel closes the
/// write end and the child's read returns EOF instantly. Strictly stronger
/// than the PID watch: zero latency, no polling, no PID-reuse caveat, and no
/// dependence on parent/ppid semantics. Set by the adapter for every
/// `__vm-boot` VM it launches.
pub const LIFELINE_FD_ENV: &str = "III_LIFELINE_FD";

/// Env var naming the SPAWNER's pid, set alongside [`LIFELINE_FD_ENV`] by
/// [`attach_lifeline`]. Children use it as the polling backstop for the one
/// lifeline failure mode: on macOS `pipe2` doesn't exist, so a concurrent
/// spawn on another thread can inherit a write end in the window before the
/// CLOEXEC fcntls land, delaying EOF until that bystander also exits. The
/// pid poll is immune to leaked fds.
pub const LIFELINE_SPAWNER_PID_ENV: &str = "III_LIFELINE_SPAWNER_PID";

/// Spawn-time facts captured BEFORE the tokio runtime exists. Reading the
/// lifeline fd out of the env requires `env::remove_var` (so children never
/// inherit it), which is only sound while the process is single-threaded —
/// `#[tokio::main]`-style entrypoints spawn worker threads before any user
/// code runs, so `main()` must call [`capture_early`] first, pre-runtime.
struct EarlyCapture {
    #[cfg(unix)]
    lifeline: std::sync::Mutex<Option<std::os::fd::OwnedFd>>,
    #[cfg(unix)]
    spawner_pid: Option<i32>,
}

static EARLY: std::sync::OnceLock<EarlyCapture> = std::sync::OnceLock::new();

/// Capture (and scrub from the env) the inherited lifeline facts. MUST be
/// the first thing `main()` does, before building the tokio runtime — this
/// is the only point where mutating the process environment is sound.
/// Idempotent; later calls are no-ops.
pub fn capture_early() {
    let _ = EARLY.get_or_init(|| EarlyCapture {
        #[cfg(unix)]
        lifeline: std::sync::Mutex::new(lifeline_fd_from_env()),
        #[cfg(unix)]
        spawner_pid: {
            let pid = std::env::var(LIFELINE_SPAWNER_PID_ENV)
                .ok()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .filter(|p| *p > 1);
            if std::env::var_os(LIFELINE_SPAWNER_PID_ENV).is_some() {
                // SAFETY: pre-runtime, single-threaded (the contract of this
                // function); scrubbed so children never inherit it.
                unsafe { std::env::remove_var(LIFELINE_SPAWNER_PID_ENV) };
            }
            pid
        },
    });
}

/// Take the early-captured lifeline read end (None if [`capture_early`]
/// wasn't called or no valid lifeline was inherited).
#[cfg(unix)]
pub fn take_early_lifeline() -> Option<std::os::fd::OwnedFd> {
    EARLY.get()?.lifeline.lock().ok()?.take()
}

/// The early-captured spawner pid, for the polling backstop.
#[cfg(unix)]
pub fn early_spawner_pid() -> Option<i32> {
    EARLY.get()?.spawner_pid
}

/// Polling-backstop companion to [`blocking_wait_lifeline_eof`]: block until
/// `pid` no longer exists (ESRCH). For plain-thread contexts like
/// `__vm-boot`.
#[cfg(unix)]
pub fn blocking_wait_pid_gone(pid: i32) {
    loop {
        std::thread::sleep(SPAWNER_POLL_INTERVAL);
        if process_is_gone(pid) {
            return;
        }
    }
}

/// How often [`blocking_wait_pid_gone`] polls the spawner.
#[cfg(unix)]
pub const SPAWNER_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// One end of a spawner→child death-notification pipe. The holder is the
/// SPAWNER: dropping a `Lifeline` (or the spawner dying, however abruptly)
/// closes the write end and the child's lifeline watch sees EOF. Created by
/// [`attach_lifeline`].
#[derive(Debug)]
pub struct Lifeline {
    /// Held open, never written. Kernel closes it on process death.
    #[cfg(unix)]
    _write: std::os::fd::OwnedFd,
    /// The parent's copy of the read end. Kept so the raw fd number baked
    /// into the child's env/pre_exec stays valid until (and after) spawn;
    /// holding a READ end open never delays the child's EOF (EOF fires when
    /// all WRITE ends close).
    #[cfg(unix)]
    _read: std::os::fd::OwnedFd,
}

/// Create a lifeline pipe and wire `cmd` so the child inherits the read end:
/// both ends are CLOEXEC in this process (no leak into other children); a
/// `pre_exec` hook clears CLOEXEC on the read end for THIS child only, and
/// [`LIFELINE_FD_ENV`] tells the child which fd number it is.
///
/// Keep the returned [`Lifeline`] alive for as long as the child should
/// live; drop it to tell the child its spawner is gone.
///
/// macOS has no `pipe2`, so there is a tiny window between `pipe()` and the
/// CLOEXEC fcntls where a concurrent spawn on another thread could inherit
/// both ends, which would delay the child's EOF until that bystander also
/// exits. The pid-poll backstop covers that case: [`LIFELINE_SPAWNER_PID_ENV`]
/// is set here and consumed by `__vm-boot`'s [`blocking_wait_pid_gone`]
/// thread.
#[cfg(unix)]
pub fn attach_lifeline(cmd: &mut tokio::process::Command) -> std::io::Result<Lifeline> {
    let (read, write) = new_cloexec_pipe()?;
    let read_raw = std::os::fd::AsRawFd::as_raw_fd(&read);
    cmd.env(LIFELINE_FD_ENV, read_raw.to_string());
    cmd.env(LIFELINE_SPAWNER_PID_ENV, std::process::id().to_string());
    // SAFETY: fcntl(F_SETFD) is async-signal-safe; runs in the
    // forked-but-not-yet-execed child to un-CLOEXEC its inherited copy.
    unsafe {
        cmd.pre_exec(move || clear_cloexec(read_raw));
    }
    Ok(Lifeline {
        _write: write,
        _read: read,
    })
}

/// Pipe with both ends CLOEXEC: atomically via `pipe2` where it exists,
/// pipe+fcntl elsewhere (see the race note on [`attach_lifeline`]).
#[cfg(unix)]
fn new_cloexec_pipe() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    #[cfg(target_os = "linux")]
    {
        let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
        Ok((r, w))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (r, w) = nix::unistd::pipe()?;
        set_cloexec(std::os::fd::AsRawFd::as_raw_fd(&r))?;
        set_cloexec(std::os::fd::AsRawFd::as_raw_fd(&w))?;
        Ok((r, w))
    }
}

#[cfg(unix)]
fn set_cloexec(fd: i32) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn clear_cloexec(fd: i32) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Child side: take ownership of the lifeline read end named by
/// [`LIFELINE_FD_ENV`], if any. Validates the fd is a live FIFO before
/// trusting it (a stale/corrupt env naming, say, stdin would otherwise turn
/// "read /dev/null EOF" into a false spawner-death). On success the fd gets
/// CLOEXEC set again and the env var is scrubbed, so OUR children inherit
/// neither: a VM must die with its own spawner, not with ours.
///
/// Single-call contract: takes ownership of the raw fd, so call at most once
/// per process ([`capture_early`] does).
#[cfg(unix)]
pub fn lifeline_fd_from_env() -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let val = std::env::var(LIFELINE_FD_ENV).ok()?;
    // Scrub BEFORE validating: every rejection path below must also keep the
    // var away from our children, not just the success path.
    // SAFETY: startup-time, before this process spawns anything that could
    // race the env table (capture_early's pre-runtime contract).
    unsafe { std::env::remove_var(LIFELINE_FD_ENV) };
    let Ok(raw) = val.trim().parse::<i32>() else {
        tracing::warn!(value = ?val, "ignoring unparsable lifeline env");
        return None;
    };
    if raw <= 2 {
        tracing::warn!(fd = raw, "ignoring lifeline env naming a stdio fd");
        return None;
    }
    // Validate BEFORE taking ownership: if the env lies and names a foreign
    // fd, we must not adopt (and later close) something that isn't ours.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(raw, &mut stat) } != 0 {
        tracing::warn!(fd = raw, "ignoring lifeline env: fd is not open");
        return None;
    }
    if stat.st_mode & libc::S_IFMT != libc::S_IFIFO {
        tracing::warn!(fd = raw, "ignoring lifeline env: fd is not a pipe");
        return None;
    }
    // Ownership BEFORE the final fcntl: a failure there must close the
    // (genuinely ours, per protocol) fd rather than leak it open and
    // non-CLOEXEC into every child we spawn.
    // SAFETY: protocol gives this process sole ownership of the inherited fd.
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    if let Err(e) = set_cloexec(raw) {
        tracing::warn!(fd = raw, error = %e, "ignoring lifeline: cannot re-arm CLOEXEC");
        return None; // drop(owned) closes it
    }
    Some(owned)
}

/// Block until the lifeline reports the spawner is gone. Returns `true` on
/// EOF (all write ends closed — the spawner died or dropped its
/// [`Lifeline`]); `false` on an unrecoverable read error, which is NOT death
/// evidence (callers fall back to their other watches). Any bytes received
/// are ignored — the protocol never writes.
#[cfg(unix)]
pub fn blocking_wait_lifeline_eof(fd: std::os::fd::OwnedFd) -> bool {
    use std::io::Read;
    let mut file = std::fs::File::from(fd);
    let mut buf = [0u8; 16];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                tracing::warn!(error = %e, "lifeline read failed; falling back to PID watches");
                return false;
            }
        }
    }
}

/// Resolve on SIGINT, SIGTERM or SIGHUP and return which ("sigint" |
/// "sigterm" | "sighup"). The handlers are installed before the
/// "shutdown signals armed" log line, so a signal sent after it is never
/// lost to the default disposition.
pub async fn wait_for_shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::SignalKind;
        let mut int = install_signal(SignalKind::interrupt(), "SIGINT");
        let mut term = install_signal(SignalKind::terminate(), "SIGTERM");
        let mut hup = install_signal(SignalKind::hangup(), "SIGHUP");
        tracing::info!("shutdown signals armed");
        tokio::select! {
            _ = recv_signal(&mut int, "SIGINT") => "sigint",
            _ = recv_signal(&mut term, "SIGTERM") => "sigterm",
            _ = recv_signal(&mut hup, "SIGHUP") => "sighup",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "sigint"
    }
}

/// A handler that cannot be installed logs and leaves its arm disabled: an
/// installation error is not an exit request, and the other arms still stop
/// the adapter.
#[cfg(unix)]
fn install_signal(
    kind: tokio::signal::unix::SignalKind,
    name: &'static str,
) -> Option<tokio::signal::unix::Signal> {
    match tokio::signal::unix::signal(kind) {
        Ok(sig) => Some(sig),
        Err(e) => {
            tracing::error!(error = %e, signal = name, "failed to install signal handler; exit arm disabled");
            None
        }
    }
}

/// Resolve when `sig` is delivered; park forever when its arm is disabled or
/// its stream ends.
#[cfg(unix)]
async fn recv_signal(sig: &mut Option<tokio::signal::unix::Signal>, name: &'static str) {
    if let Some(sig) = sig {
        if sig.recv().await.is_some() {
            return;
        }
        tracing::error!(signal = name, "signal stream closed; exit arm disabled");
    }
    std::future::pending::<()>().await
}

/// Existence probe via `kill(pid, 0)`: true only on ESRCH (no such process).
/// EPERM means the process exists but isn't ours — counts as alive.
#[cfg(unix)]
fn process_is_gone(pid: i32) -> bool {
    matches!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::process_is_gone;

    #[test]
    fn process_is_gone_only_on_esrch() {
        // Our own process and our own parent demonstrably exist.
        assert!(!process_is_gone(std::process::id() as i32));
        assert!(!process_is_gone(nix::unistd::getppid().as_raw()));
        // PID 1 (launchd/init) always exists; we can't signal it → EPERM →
        // counts as alive, never as gone.
        assert!(!process_is_gone(1));
        // A child we spawn and fully reap is guaranteed dead at probe time.
        // (Its pid could in principle be recycled between reap and probe, in
        // which case the probe correctly says "exists" — so don't assert the
        // negative; just that the probe never panics.)
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        child.wait().unwrap();
        let _ = process_is_gone(pid);
    }

    /// Serializes the env-mutating tests below: `lifeline_fd_from_env` takes
    /// fd ownership, so unguarded parallel runs could double-own an fd or see
    /// each other's vars.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn lifeline_env_rejects_garbage_and_non_pipes() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        unsafe { std::env::remove_var(super::LIFELINE_FD_ENV) };
        assert!(super::lifeline_fd_from_env().is_none());

        // Stdio fd numbers are refused outright.
        unsafe { std::env::set_var(super::LIFELINE_FD_ENV, "0") };
        assert!(super::lifeline_fd_from_env().is_none());

        // A live fd that is NOT a pipe (a regular file) is refused — a
        // corrupt env must not turn "EOF on some file" into spawner death.
        let f = std::fs::File::open("/dev/null").unwrap();
        let raw = std::os::fd::AsRawFd::as_raw_fd(&f);
        unsafe { std::env::set_var(super::LIFELINE_FD_ENV, raw.to_string()) };
        assert!(super::lifeline_fd_from_env().is_none());

        // Closed/garbage fd numbers are refused.
        unsafe { std::env::set_var(super::LIFELINE_FD_ENV, "9999") };
        assert!(super::lifeline_fd_from_env().is_none());
        // The var is scrubbed even on rejection (children must not inherit).
        assert!(std::env::var(super::LIFELINE_FD_ENV).is_err());
    }

    #[test]
    fn lifeline_accepts_pipe_and_sees_eof_when_writer_drops() {
        use std::os::fd::IntoRawFd;
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let (read, write) = nix::unistd::pipe().unwrap();
        // Hand the raw number over as "inherited" — lifeline_fd_from_env
        // takes ownership, so release ours first.
        let raw = read.into_raw_fd();
        unsafe { std::env::set_var(super::LIFELINE_FD_ENV, raw.to_string()) };
        let owned = super::lifeline_fd_from_env().expect("pipe read end accepted");
        // Env scrubbed so OUR children can't inherit the lifeline.
        assert!(std::env::var(super::LIFELINE_FD_ENV).is_err());

        // EOF only after the write end drops.
        let watcher = std::thread::spawn(move || super::blocking_wait_lifeline_eof(owned));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!watcher.is_finished(), "no EOF while the writer is alive");
        drop(write);
        assert!(
            watcher.join().unwrap(),
            "dropping the write end must read as EOF (spawner gone)"
        );
    }
}
