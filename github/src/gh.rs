//! Process core for the `gh` CLI: spawn with bounded capture, a timeout
//! watchdog, and process-group kill. The bounded-read / group-kill / stdin
//! mechanics are lifted from `shell/src/exec/host.rs` (see the comments there
//! for the full rationale); this file trims the policy jail, env scrubbing,
//! and background-job machinery a gh wrapper doesn't need.

use std::process::Stdio;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::Config;

/// A completed `gh` invocation. This is also the `github::exec` response: a
/// non-zero exit or a timeout is DATA here — only failure to run the process
/// at all is a `GhError`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GhOutcome {
    /// Captured stdout (UTF-8 lossy), capped at `max_output_bytes`.
    pub stdout: String,
    /// Captured stderr (UTF-8 lossy), capped at `max_output_bytes`.
    pub stderr: String,
    /// Process exit code; null when the process was killed (timeout).
    pub exit_code: Option<i32>,
    /// Wall-clock duration of the invocation, in ms.
    pub duration_ms: u64,
    /// True when the timeout expired and the process group was killed.
    pub timed_out: bool,
    /// True when stdout exceeded `max_output_bytes` and was truncated.
    pub stdout_truncated: bool,
    /// True when stderr exceeded `max_output_bytes` and was truncated.
    pub stderr_truncated: bool,
}

/// Machine-branchable failure to run gh at all. Lifted to
/// `Error::Remote { code, message }` so a caller can branch on `error.code`
/// (`gh_not_found`, `spawn_failed`, `timeout`) instead of parsing messages —
/// any other `Error` variant collapses to `code: "invocation_failed"` on the
/// wire (see shell's `ExecError` for the precedent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhError {
    pub code: &'static str,
    pub message: String,
}

impl GhError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn timeout(ms: u64) -> Self {
        Self::new("timeout", format!("gh timed out after {ms} ms"))
    }
}

impl From<GhError> for iii_sdk::errors::Error {
    fn from(err: GhError) -> Self {
        iii_sdk::errors::Error::Remote {
            code: err.code.to_string(),
            message: err.message,
            stacktrace: None,
        }
    }
}

/// Run `gh` with `args`, bounded capture, and a timeout watchdog. `timeout_ms`
/// is clamped via [`Config::resolve_timeout`]. A timeout is returned as
/// `timed_out: true` (with the whole process group killed), not as an error —
/// `github::exec` exposes it as data; the typed wrappers lift it to
/// `GhError::timeout`.
pub async fn run(
    cfg: &Config,
    args: &[String],
    stdin: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<GhOutcome, GhError> {
    let started = std::time::Instant::now();
    let mut cmd = build_command(cfg, args, stdin.is_some());
    let mut child = cmd.spawn().map_err(|e| spawn_error(&cfg.gh_bin(), &e))?;
    pump_stdin(&mut child, &stdin);

    let mut stdout_reader = child
        .stdout
        .take()
        .ok_or_else(|| GhError::new("spawn_failed", "no stdout pipe"))?;
    let mut stderr_reader = child
        .stderr
        .take()
        .ok_or_else(|| GhError::new("spawn_failed", "no stderr pipe"))?;

    let limit = cfg.max_output_bytes;
    let timeout = Duration::from_millis(cfg.resolve_timeout(timeout_ms));

    let stdout_task = tokio::spawn(async move { read_bounded(&mut stdout_reader, limit).await });
    let stderr_task = tokio::spawn(async move { read_bounded(&mut stderr_reader, limit).await });

    let wait_res = tokio::time::timeout(timeout, child.wait()).await;

    let (exit_code, timed_out) = match wait_res {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(GhError::new("spawn_failed", format!("wait: {e}"))),
        Err(_) => {
            // Hard timeout: kill the whole process group (grandchildren too,
            // e.g. the git/credential helpers gh spawns) while we still own
            // the un-reaped child, then reap. start_kill is the non-unix
            // fallback (kill_process_group is a no-op there).
            kill_process_group(child.id());
            let _ = child.start_kill();
            let _ = child.wait().await;
            (None, true)
        }
    };

    let (stdout_bytes, stdout_truncated) = stdout_task
        .await
        .map_err(|e| GhError::new("spawn_failed", format!("stdout: {e}")))?;
    let (stderr_bytes, stderr_truncated) = stderr_task
        .await
        .map_err(|e| GhError::new("spawn_failed", format!("stderr: {e}")))?;

    Ok(GhOutcome {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        timed_out,
        stdout_truncated,
        stderr_truncated,
    })
}

fn spawn_error(bin: &str, e: &std::io::Error) -> GhError {
    if e.kind() == std::io::ErrorKind::NotFound {
        GhError::new(
            "gh_not_found",
            format!(
                "gh executable '{bin}' not found; install the GitHub CLI \
                 (https://cli.github.com) or set gh_executable in the github configuration"
            ),
        )
    } else {
        GhError::new("spawn_failed", format!("spawn {bin}: {e}"))
    }
}

fn build_command(cfg: &Config, args: &[String], stdin: bool) -> Command {
    let mut cmd = Command::new(cfg.gh_bin());
    cmd.args(args);
    // Never let gh block on an interactive prompt or chat about updates.
    cmd.env("GH_PROMPT_DISABLED", "1");
    cmd.env("GH_NO_UPDATE_NOTIFIER", "1");
    // A non-empty configured token wins; empty inherits the worker's env
    // (ambient `gh auth login` state or an already-exported GH_TOKEN).
    if !cfg.token.is_empty() {
        cmd.env("GH_TOKEN", &cfg.token);
    }
    // stdin is piped when the caller supplied bytes (the spawn site writes them
    // then closes the pipe so the child sees EOF); otherwise /dev/null so a
    // command that reads stdin doesn't block forever.
    cmd.stdin(if stdin { Stdio::piped() } else { Stdio::null() });
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Tokio's Command does NOT kill children on drop by default; without
        // this a worker shutdown would orphan a still-running gh.
        .kill_on_drop(true);
    // Put the child in its own process group (it becomes the group leader, so
    // pgid == pid). Termination then signals the WHOLE group via `-pid`,
    // killing any grandchildren too — not just the direct child, which
    // `start_kill`/`kill_on_drop` alone would leave orphaned.
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

/// SIGKILL an entire process group by its leader pid. The caller MUST still own
/// the un-reaped `Child` for `pid` when calling this, so the pid (and therefore
/// the pgid, since children are spawned with `process_group(0)`) cannot have
/// been reused by the OS — this is what makes signalling by number safe. The
/// negated pid targets the whole group, terminating grandchildren too. No-op on
/// non-unix; a vanished process (ESRCH) is ignored.
#[cfg(unix)]
fn kill_process_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: see the doc above — `pid` is an un-reaped child we own, and
        // the negative value signals its process group.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: Option<u32>) {}

/// Feed `stdin` (if any) to the child's stdin pipe, then close it so the child
/// sees EOF. Runs in a detached task so a child that interleaves reading stdin
/// with writing stdout cannot deadlock against our stdout/stderr drains
/// (each side would otherwise block on a full pipe). Best-effort: a child that
/// exited before consuming its input just makes the write fail, which we ignore.
fn pump_stdin(child: &mut tokio::process::Child, stdin: &Option<String>) {
    if let Some(data) = stdin {
        if let Some(mut sin) = child.stdin.take() {
            let bytes = data.clone().into_bytes();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = sin.write_all(&bytes).await;
                let _ = sin.shutdown().await;
            });
        }
    }
}

async fn read_bounded<R: AsyncReadExt + Unpin>(reader: &mut R, limit: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if !truncated {
                    if buf.len() + n > limit {
                        let take = limit.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..take]);
                        truncated = true;
                        // Fall through and continue draining; do NOT break.
                        // Dropping the reader here closes the pipe, and the
                        // child's next write() raises SIGPIPE — making
                        // ExitStatus::code() report None on what should have
                        // been a normal-exit-with-truncated-output run.
                    } else {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
                // truncated == true: discard everything past `limit`.
            }
            Err(_) => break,
        }
    }
    (buf, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point `gh_executable` at a plain binary so the mechanics are testable
    /// without gh installed (CI runners don't have it).
    fn cfg_for(bin: &str) -> Config {
        Config {
            gh_executable: bin.to_string(),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn echo_roundtrip() {
        let out = run(&cfg_for("echo"), &["hi".into()], None, None)
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "hi");
        assert!(!out.timed_out);
        assert!(!out.stdout_truncated);
    }

    #[tokio::test]
    async fn missing_binary_is_gh_not_found() {
        let err = run(&cfg_for("/nonexistent/gh-not-here"), &[], None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, "gh_not_found");
        assert!(err.message.contains("cli.github.com"));
    }

    #[tokio::test]
    async fn timeout_kills_and_reports_as_data() {
        let out = run(&cfg_for("sleep"), &["5".into()], None, Some(200))
            .await
            .unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, None);
    }

    #[tokio::test]
    async fn truncation_keeps_the_real_exit_code() {
        let mut cfg = cfg_for("sh");
        cfg.max_output_bytes = 16;
        let out = run(
            &cfg,
            &["-c".into(), "printf 'x%.0s' $(seq 1 100)".into()],
            None,
            None,
        )
        .await
        .unwrap();
        assert!(out.stdout_truncated);
        assert_eq!(out.stdout.len(), 16);
        // drain-after-cap: the child still exits cleanly (no SIGPIPE kill)
        assert_eq!(out.exit_code, Some(0));
    }

    #[tokio::test]
    async fn stdin_reaches_the_child() {
        let out = run(&cfg_for("cat"), &[], Some("ping".into()), None)
            .await
            .unwrap();
        assert_eq!(out.stdout, "ping");
        assert_eq!(out.exit_code, Some(0));
    }

    #[tokio::test]
    async fn token_is_injected_into_the_child_env() {
        let mut cfg = cfg_for("sh");
        cfg.token = "tkn-123".into();
        let out = run(
            &cfg,
            &["-c".into(), "printf %s \"$GH_TOKEN\"".into()],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.stdout, "tkn-123");
    }

    #[tokio::test]
    async fn nonzero_exit_is_data() {
        let out = run(
            &cfg_for("sh"),
            &["-c".into(), "echo boom >&2; exit 3".into()],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.exit_code, Some(3));
        assert_eq!(out.stderr.trim(), "boom");
        assert!(!out.timed_out);
    }
}
