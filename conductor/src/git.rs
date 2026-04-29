use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct CmdResult {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub async fn run_cmd(cwd: &Path, bin: &str, args: &[&str], timeout_ms: Option<u64>) -> CmdResult {
    let mut command = Command::new(bin);
    command.current_dir(cwd).args(args);
    command.kill_on_drop(true);

    let fut = command.output();
    let output = match timeout_ms {
        Some(ms) => match timeout(Duration::from_millis(ms), fut).await {
            Ok(out) => out,
            Err(_) => {
                return CmdResult {
                    ok: false,
                    code: None,
                    stdout: String::new(),
                    stderr: format!("timed out after {ms}ms"),
                };
            }
        },
        None => fut.await,
    };

    match output {
        Ok(o) => CmdResult {
            ok: o.status.success(),
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => CmdResult {
            ok: false,
            code: None,
            stdout: String::new(),
            stderr: format!("{e}"),
        },
    }
}

pub async fn create_worktree(
    repo_cwd: &Path,
    branch: &str,
    root: &Path,
) -> Result<PathBuf, String> {
    let safe_branch: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = root.join(&safe_branch);

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }

    let path_str = path.to_string_lossy().into_owned();
    let r = run_cmd(
        repo_cwd,
        "git",
        &["worktree", "add", "-b", branch, &path_str],
        None,
    )
    .await;
    if !r.ok {
        return Err(r.stderr.trim().to_string());
    }
    Ok(path)
}

pub async fn remove_worktree(repo_cwd: &Path, path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy().into_owned();
    let r = run_cmd(
        repo_cwd,
        "git",
        &["worktree", "remove", "--force", &path_str],
        None,
    )
    .await;
    if !r.ok {
        return Err(r.stderr.trim().to_string());
    }
    Ok(())
}

pub async fn diff_against(cwd: &Path, base_ref: &str) -> String {
    let r = run_cmd(cwd, "git", &["diff", base_ref, "--", "."], None).await;
    r.stdout
}

pub async fn current_branch(cwd: &Path) -> String {
    let r = run_cmd(cwd, "git", &["rev-parse", "--abbrev-ref", "HEAD"], None).await;
    r.stdout.trim().to_string()
}
