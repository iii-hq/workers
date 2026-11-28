//! Local tools: `git`, `gh`, `docker`, the configured `iii`. Failures are
//! coded (`GIT_FAILED`, `GH_FAILED`, `TOOL_MISSING`, `DOCKER_MISSING`) with
//! stderr cut to 2 KB.

use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::{CloudError, Res, cut};

pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Run `bin args` in `cwd`; `stdin` is fed when given. A missing binary is
/// `missing_code` (`TOOL_MISSING` unless the caller says otherwise).
pub async fn run(
    bin: &str,
    args: &[&str],
    cwd: Option<&Path>,
    stdin: Option<&[u8]>,
    missing_code: &str,
) -> Res<Out> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CloudError::new(
                missing_code,
                format!("`{bin}` is not installed or not on PATH."),
            )
        } else {
            CloudError::new(missing_code, format!("could not start `{bin}`: {e}"))
        }
    })?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(data).await;
        drop(pipe);
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| CloudError::new(missing_code, format!("`{bin}` failed: {e}")))?;
    Ok(Out {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Run a tool that must succeed; failure is `fail_code` with its stderr.
pub async fn must(
    bin: &str,
    args: &[&str],
    cwd: Option<&Path>,
    fail_code: &str,
    missing_code: &str,
) -> Res<String> {
    let out = run(bin, args, cwd, None, missing_code).await?;
    if out.ok() {
        Ok(out.stdout)
    } else {
        Err(CloudError::new(
            fail_code,
            cut(
                format!(
                    "{bin} {}: {}",
                    args.iter().find(|a| !a.starts_with('-')).unwrap_or(&""),
                    if out.stderr.trim().is_empty() {
                        out.stdout.trim()
                    } else {
                        out.stderr.trim()
                    }
                )
                .as_str(),
                2048,
            ),
        ))
    }
}

pub async fn git(cwd: &Path, args: &[&str]) -> Res<String> {
    must("git", args, Some(cwd), "GIT_FAILED", "TOOL_MISSING").await
}

pub async fn gh(cwd: &Path, args: &[&str]) -> Res<String> {
    must("gh", args, Some(cwd), "GH_FAILED", "GH_FAILED").await
}
