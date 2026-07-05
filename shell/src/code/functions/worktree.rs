//! `coder::worktree-add` / `coder::worktree-remove` — git worktree
//! lifecycle for isolated sub-agent workspaces. A worktree lives at
//! `<root>/.worktrees/<name>` on branch `wt/<name>`, where `<root>` is the
//! call's effective root (the harness-stamped fs_scope root, else the
//! primary allowed root). Removal is clean-only: a dirty worktree is left
//! in place and reported, never force-deleted.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::path::PathResolver;

/// Wall-clock budget per git invocation (worktree add checks out files).
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Directory (under the effective root) that holds the worktrees.
const WORKTREES_DIR: &str = ".worktrees";
/// Branch prefix for worktree branches.
const BRANCH_PREFIX: &str = "wt/";

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_worktree_add_input")]
pub struct WorktreeAddInput {
    /// Worktree name — letters, digits, `-` and `_` only. The worktree is
    /// created at `.worktrees/<name>` under the effective root, on a new
    /// branch `wt/<name>` from the current HEAD.
    pub name: String,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_worktree_add_input() -> serde_json::Value {
    serde_json::json!({ "name": "fix-auth-bug-k4x2" })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeAddOutput {
    /// Canonical absolute path of the new worktree.
    pub path: String,
    /// The branch the worktree is on (`wt/<name>`), from the root's HEAD.
    pub branch: String,
}

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_worktree_remove_input")]
pub struct WorktreeRemoveInput {
    /// Name of the worktree to remove (the `.worktrees/<name>` entry).
    pub name: String,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_worktree_remove_input() -> serde_json::Value {
    serde_json::json!({ "name": "fix-auth-bug-k4x2" })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeRemoveOutput {
    /// True when the worktree directory was removed.
    pub removed: bool,
    /// True when removal was refused because the worktree has uncommitted
    /// changes — inspect or commit them, the path is untouched.
    pub dirty: bool,
    /// Canonical absolute path of the (former) worktree.
    pub path: String,
    /// The worktree's branch (`wt/<name>`). Kept when it holds unmerged
    /// commits; deleted with the worktree only when fully merged.
    pub branch: String,
    /// True when `branch` was deleted along with the worktree.
    pub branch_deleted: bool,
}

pub async fn handle_add(
    resolver: Arc<PathResolver>,
    _cfg: Arc<CoderConfig>,
    req: WorktreeAddInput,
) -> Result<WorktreeAddOutput, String> {
    let root = resolver.effective_root(crate::fs::scope_root(req.fs_scope.as_ref()));
    let name = validate_name(&req.name).map_err(err_to_string)?;
    require_git_worktree(&root).await.map_err(err_to_string)?;

    let rel = format!("{WORKTREES_DIR}/{name}");
    let branch = format!("{BRANCH_PREFIX}{name}");
    let out = run_git(&root, &["worktree", "add", "-b", &branch, &rel, "HEAD"])
        .await
        .map_err(err_to_string)?;
    if !out.status.success() {
        return Err(err_to_string(CoderError::BadInput(format!(
            "git worktree add failed: {} — a worktree or branch named \
             {name:?} may already exist; pick a different name",
            String::from_utf8_lossy(&out.stderr).trim()
        ))));
    }
    let path = root.join(&rel);
    let path = path.canonicalize().unwrap_or(path);
    Ok(WorktreeAddOutput {
        path: path.display().to_string(),
        branch,
    })
}

pub async fn handle_remove(
    resolver: Arc<PathResolver>,
    _cfg: Arc<CoderConfig>,
    req: WorktreeRemoveInput,
) -> Result<WorktreeRemoveOutput, String> {
    let root = resolver.effective_root(crate::fs::scope_root(req.fs_scope.as_ref()));
    let name = validate_name(&req.name).map_err(err_to_string)?;
    require_git_worktree(&root).await.map_err(err_to_string)?;

    let rel = format!("{WORKTREES_DIR}/{name}");
    let branch = format!("{BRANCH_PREFIX}{name}");
    let wt_path = root.join(&rel);
    let wt_display = wt_path
        .canonicalize()
        .unwrap_or_else(|_| wt_path.clone())
        .display()
        .to_string();
    if !wt_path.is_dir() {
        return Err(err_to_string(CoderError::BadInput(format!(
            "no worktree at {WORKTREES_DIR}/{name} under the effective root"
        ))));
    }

    // Clean-only: uncommitted changes keep the worktree in place.
    let status = run_git(&wt_path, &["status", "--porcelain"])
        .await
        .map_err(err_to_string)?;
    if !status.status.success() {
        return Err(err_to_string(CoderError::Io(format!(
            "git status failed in {wt_display}: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ))));
    }
    if !status.stdout.is_empty() {
        return Ok(WorktreeRemoveOutput {
            removed: false,
            dirty: true,
            path: wt_display,
            branch,
            branch_deleted: false,
        });
    }

    let rm = run_git(&root, &["worktree", "remove", &rel])
        .await
        .map_err(err_to_string)?;
    if !rm.status.success() {
        return Err(err_to_string(CoderError::Io(format!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&rm.stderr).trim()
        ))));
    }
    // `-d` only deletes a fully-merged branch — an unmerged branch (the
    // child's unlanded work) survives for the parent to merge.
    let branch_deleted = run_git(&root, &["branch", "-d", &branch])
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    Ok(WorktreeRemoveOutput {
        removed: true,
        dirty: false,
        path: wt_display,
        branch,
        branch_deleted,
    })
}

fn validate_name(name: &str) -> Result<&str, CoderError> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CoderError::BadInput(format!(
            "invalid worktree name {name:?} — use only letters, digits, \
             '-' and '_' (max 128 chars)"
        )));
    }
    Ok(name)
}

async fn require_git_worktree(root: &Path) -> Result<(), CoderError> {
    let out = run_git(root, &["rev-parse", "--is-inside-work-tree"]).await?;
    if !out.status.success() {
        return Err(CoderError::BadInput(format!(
            "{} is not inside a git work tree — worktree isolation needs a \
             git repository",
            root.display()
        )));
    }
    Ok(())
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<std::process::Output, CoderError> {
    let fut = tokio::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output();
    match tokio::time::timeout(GIT_TIMEOUT, fut).await {
        Err(_) => Err(CoderError::Io(format!(
            "git {} timed out after {}s",
            args.first().unwrap_or(&""),
            GIT_TIMEOUT.as_secs()
        ))),
        Ok(Err(e)) => Err(CoderError::Io(format!("failed to run git: {e}"))),
        Ok(Ok(out)) => Ok(out),
    }
}
