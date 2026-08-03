//! Hardened `git` subprocess runner. Mirrors the iii-directory git source:
//! prompts disabled, user-controlled transports disabled, `ext::` transport
//! never allowed, every call bounded by a tokio timeout, and every
//! caller-supplied value guarded against argument injection.

pub mod ops;
pub mod porcelain;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::error::{codes, WError};

#[derive(Debug, Clone)]
pub struct GitOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GitOutput {
    pub fn stdout_trimmed(&self) -> String {
        self.stdout.trim().to_string()
    }
}

/// The single hardened spawn point: prompts disabled, ext transport never
/// allowed, bounded by one timeout, output captured whatever the exit code.
async fn run_git_inner(
    dir: &Path,
    args: &[&str],
    stdin_data: Option<&[u8]>,
    timeout_ms: u64,
) -> Result<GitOutput, WError> {
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("protocol.ext.allow=never")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let run = async {
        let mut child = cmd
            .spawn()
            .map_err(|e| WError::new(codes::GIT_SPAWN, format!("spawn git: {e}")))?;
        if let Some(data) = stdin_data {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin
                    .write_all(data)
                    .await
                    .map_err(|e| WError::new(codes::GIT_SPAWN, format!("write git stdin: {e}")))?;
                drop(stdin);
            }
        }
        child
            .wait_with_output()
            .await
            .map_err(|e| WError::new(codes::GIT_SPAWN, format!("await git: {e}")))
    };
    let output = timeout(Duration::from_millis(timeout_ms), run)
        .await
        .map_err(|_| {
            WError::new(
                codes::GIT_TIMEOUT,
                format!("git {} timed out after {timeout_ms} ms", args.join(" ")),
            )
        })??;

    Ok(GitOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Run `git <args>` in `dir` with hardening and a timeout. Returns the
/// captured output regardless of exit code; use [`run_git_ok`] when nonzero
/// is an error.
pub async fn run_git(dir: &Path, args: &[&str], timeout_ms: u64) -> Result<GitOutput, WError> {
    run_git_inner(dir, args, None, timeout_ms).await
}

/// Like [`run_git`], but feeds `stdin_data` to the child (used for the
/// `diff-tree --stdin` / `patch-id` plumbing pipelines).
pub async fn run_git_with_stdin(
    dir: &Path,
    args: &[&str],
    stdin_data: &[u8],
    timeout_ms: u64,
) -> Result<GitOutput, WError> {
    run_git_inner(dir, args, Some(stdin_data), timeout_ms).await
}

/// Like [`run_git`], but a nonzero exit is a `W102` error carrying the
/// trimmed stderr.
pub async fn run_git_ok(dir: &Path, args: &[&str], timeout_ms: u64) -> Result<GitOutput, WError> {
    let out = run_git(dir, args, timeout_ms).await?;
    if out.exit_code != 0 {
        return Err(WError::new(
            codes::GIT_NONZERO,
            format!(
                "git {} exited with {}: {}",
                args.join(" "),
                out.exit_code,
                out.stderr.trim()
            ),
        ));
    }
    Ok(out)
}

/// Canonicalize a path, falling back to the path itself when resolution
/// fails (missing entry, permissions). Canonical spellings keep comparisons
/// against git's own output honest: git reports resolved paths while
/// callers may use a symlinked spelling (macOS /var vs /private/var).
pub fn canonical_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Reject values that would be parsed as git options.
pub fn validate_no_dash(label: &str, value: &str) -> Result<(), WError> {
    if value.trim().is_empty() {
        return Err(WError::new(
            codes::BAD_PATH,
            format!("{label} must be non-empty"),
        ));
    }
    if value.starts_with('-') {
        return Err(WError::new(
            codes::BAD_PATH,
            format!("{label} may not start with '-' (argument injection): {value:?}"),
        ));
    }
    Ok(())
}

/// A caller-supplied filesystem path: absolute, UTF-8, no injection.
pub fn validate_abs_path(label: &str, value: &str) -> Result<PathBuf, WError> {
    validate_no_dash(label, value)?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(WError::new(
            codes::BAD_PATH,
            format!("{label} must be an absolute path: {value:?}"),
        ));
    }
    Ok(path)
}

/// A caller-supplied ref or branch name: conservative charset, no traversal,
/// no injection. Stricter than `git check-ref-format`, never looser.
pub fn validate_ref_name(label: &str, value: &str) -> Result<(), WError> {
    validate_no_dash(label, value)?;
    if value.contains("..")
        || value.ends_with(".lock")
        || value.ends_with('/')
        || value.starts_with('/')
        || value.contains("//")
        || value.ends_with('.')
    {
        return Err(WError::new(
            codes::BAD_PATH,
            format!("{label} is not a valid ref name: {value:?}"),
        ));
    }
    for c in value.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | '@' | '+');
        if !ok {
            return Err(WError::new(
                codes::BAD_PATH,
                format!("{label} contains an unsupported character {c:?}: {value:?}"),
            ));
        }
    }
    Ok(())
}

/// Boot-time probe: warn when the host git is older than 2.35 (worktree
/// lock reasons and porcelain behavior this worker relies on).
pub async fn probe_git_version(timeout_ms: u64) {
    let dir = std::env::temp_dir();
    match run_git(&dir, &["--version"], timeout_ms).await {
        Ok(out) if out.exit_code == 0 => {
            let line = out.stdout_trimmed();
            let version = line.split_whitespace().nth(2).unwrap_or("");
            let mut parts = version.split('.');
            let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            if (major, minor) < (2, 35) {
                tracing::warn!(
                    version,
                    "git older than 2.35 detected; worktree locking and porcelain \
                     parsing may misbehave"
                );
            } else {
                tracing::info!(version, "git probe ok");
            }
        }
        Ok(out) => tracing::warn!(stderr = %out.stderr.trim(), "git --version exited nonzero"),
        Err(e) => tracing::warn!(error = %e, "git probe failed; git operations will not work"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dash_prefixed_values() {
        assert!(validate_no_dash("ref", "--upload-pack=evil").is_err());
        assert!(validate_no_dash("ref", "").is_err());
        assert!(validate_no_dash("ref", "main").is_ok());
    }

    #[test]
    fn rejects_relative_paths() {
        assert!(validate_abs_path("repo_path", "relative/path").is_err());
        assert!(validate_abs_path("repo_path", "/abs/path").is_ok());
    }

    #[test]
    fn ref_name_charset_is_conservative() {
        assert!(validate_ref_name("branch", "iii/wt_ab12cd34").is_ok());
        assert!(validate_ref_name("branch", "feature/x.y+z@1").is_ok());
        assert!(validate_ref_name("branch", "a..b").is_err());
        assert!(validate_ref_name("branch", "a b").is_err());
        assert!(validate_ref_name("branch", "a;b").is_err());
        assert!(validate_ref_name("branch", "/lead").is_err());
        assert!(validate_ref_name("branch", "trail/").is_err());
        assert!(validate_ref_name("branch", "x.lock").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn run_git_times_out() {
        // Paused clock: the timer auto-advances past the deadline as soon as
        // the runtime parks waiting on the child, so the timeout path trips
        // deterministically instead of racing a real subprocess.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let port_arg = format!("--port={port}");
        let args = [
            "daemon",
            "--listen=127.0.0.1",
            port_arg.as_str(),
            "--reuseaddr",
        ];
        let err = run_git(&std::env::temp_dir(), &args, 1).await.unwrap_err();
        assert_eq!(err.code, crate::error::codes::GIT_TIMEOUT);
    }
}
