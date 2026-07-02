//! Git operations composed from the hardened runner. Every caller-supplied
//! value is validated before it reaches an argv.

use std::path::Path;

use crate::error::{codes, WError};
use crate::git::porcelain::{parse_status_v2, parse_worktree_list, StatusV2, WorktreeListEntry};
use crate::git::{run_git, run_git_ok};

/// Resolve a ref to a commit sha. `W112` when it does not resolve.
pub async fn rev_parse(dir: &Path, reference: &str, timeout_ms: u64) -> Result<String, WError> {
    let spec = format!("{reference}^{{commit}}");
    let out = run_git(
        dir,
        &["rev-parse", "--verify", "--quiet", &spec],
        timeout_ms,
    )
    .await?;
    if out.exit_code != 0 {
        return Err(WError::new(
            codes::REF_NOT_FOUND,
            format!(
                "ref {reference:?} does not resolve to a commit in {}",
                dir.display()
            ),
        ));
    }
    Ok(out.stdout_trimmed())
}

/// Canonicalized `git rev-parse --git-common-dir`: the per-repo key that
/// defeats symlink and worktree aliasing. `W110` when `dir` is not a repo.
pub async fn git_common_dir(dir: &Path, timeout_ms: u64) -> Result<String, WError> {
    let out = run_git(dir, &["rev-parse", "--git-common-dir"], timeout_ms).await?;
    if out.exit_code != 0 {
        return Err(WError::new(
            codes::NOT_A_REPO,
            format!(
                "{} is not inside a git repository: {}",
                dir.display(),
                out.stderr.trim()
            ),
        ));
    }
    let raw = out.stdout_trimmed();
    let path = Path::new(&raw);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    };
    let canonical = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    Ok(canonical.to_string_lossy().into_owned())
}

pub async fn worktree_add(
    repo: &Path,
    worktree_path: &Path,
    branch: &str,
    base_sha: &str,
    worktree_id: &str,
    timeout_ms: u64,
) -> Result<(), WError> {
    let reason = format!("iii:worktree {worktree_id}");
    let path_str = worktree_path.to_string_lossy();
    run_git_ok(
        repo,
        &[
            "worktree", "add", "--lock", "--reason", &reason, "-b", branch, &path_str, base_sha,
        ],
        timeout_ms,
    )
    .await
    .map(|_| ())
}

/// Idempotent unlock: "not locked" is success.
pub async fn worktree_unlock(repo: &Path, worktree_path: &Path, timeout_ms: u64) {
    let path_str = worktree_path.to_string_lossy();
    match run_git(repo, &["worktree", "unlock", &path_str], timeout_ms).await {
        Ok(out) if out.exit_code != 0 => {
            tracing::debug!(stderr = %out.stderr.trim(), "worktree unlock skipped");
        }
        Err(e) => tracing::debug!(error = %e, "worktree unlock failed"),
        _ => {}
    }
}

pub async fn worktree_remove(
    repo: &Path,
    worktree_path: &Path,
    force: bool,
    timeout_ms: u64,
) -> Result<(), WError> {
    let path_str = worktree_path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path_str);
    run_git_ok(repo, &args, timeout_ms).await.map(|_| ())
}

/// `git worktree prune` cleans stale admin entries after manual removals.
pub async fn worktree_prune(repo: &Path, timeout_ms: u64) {
    if let Err(e) = run_git_ok(repo, &["worktree", "prune"], timeout_ms).await {
        tracing::debug!(error = %e, "git worktree prune failed");
    }
}

pub async fn worktree_list(repo: &Path, timeout_ms: u64) -> Result<Vec<WorktreeListEntry>, WError> {
    let out = run_git_ok(repo, &["worktree", "list", "--porcelain"], timeout_ms).await?;
    Ok(parse_worktree_list(&out.stdout))
}

pub async fn branch_exists(repo: &Path, branch: &str, timeout_ms: u64) -> Result<bool, WError> {
    let full = format!("refs/heads/{branch}");
    let out = run_git(
        repo,
        &["show-ref", "--verify", "--quiet", &full],
        timeout_ms,
    )
    .await?;
    Ok(out.exit_code == 0)
}

/// Best-effort branch delete; `-d` unless `force`. A missing branch is a no-op.
pub async fn branch_delete(repo: &Path, branch: &str, force: bool, timeout_ms: u64) -> bool {
    let flag = if force { "-D" } else { "-d" };
    match run_git(repo, &["branch", flag, branch], timeout_ms).await {
        Ok(out) if out.exit_code == 0 => true,
        Ok(out) => {
            tracing::debug!(branch, stderr = %out.stderr.trim(), "branch delete skipped");
            false
        }
        Err(e) => {
            tracing::debug!(branch, error = %e, "branch delete failed");
            false
        }
    }
}

/// True when `ancestor` is reachable from `descendant`.
pub async fn is_ancestor(
    repo: &Path,
    ancestor: &str,
    descendant: &str,
    timeout_ms: u64,
) -> Result<bool, WError> {
    let out = run_git(
        repo,
        &["merge-base", "--is-ancestor", ancestor, descendant],
        timeout_ms,
    )
    .await?;
    // git reports the ancestry answer as exit 0 (yes) / 1 (no); any other
    // nonzero exit is a real failure (invalid ref, IO) and must propagate.
    match out.exit_code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(WError::new(
            codes::GIT_NONZERO,
            format!(
                "merge-base --is-ancestor failed in {}: {}",
                repo.display(),
                out.stderr.trim()
            ),
        )),
    }
}

pub async fn status(dir: &Path, timeout_ms: u64) -> Result<StatusV2, WError> {
    let out = run_git_ok(dir, &["status", "--porcelain=v2", "--branch"], timeout_ms).await?;
    Ok(parse_status_v2(&out.stdout))
}

/// True while a rebase is in progress in this worktree.
pub async fn is_rebase_in_progress(dir: &Path, timeout_ms: u64) -> Result<bool, WError> {
    for state_dir in ["rebase-merge", "rebase-apply"] {
        let out = run_git(dir, &["rev-parse", "--git-path", state_dir], timeout_ms).await?;
        if out.exit_code == 0 {
            let raw = out.stdout_trimmed();
            let path = Path::new(&raw);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                dir.join(path)
            };
            if absolute.exists() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Unmerged paths of an in-progress rebase.
pub async fn conflict_files(dir: &Path, timeout_ms: u64) -> Result<Vec<String>, WError> {
    let out = run_git_ok(dir, &["diff", "--name-only", "--diff-filter=U"], timeout_ms).await?;
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Rebase the worktree onto `target_sha`. Returns the raw output; a nonzero
/// exit is surfaced as `Ok(false)` so the caller can inspect the rebase state.
pub async fn rebase_onto(dir: &Path, target_sha: &str, timeout_ms: u64) -> Result<bool, WError> {
    let out = run_git(dir, &["rebase", target_sha], timeout_ms).await?;
    Ok(out.exit_code == 0)
}

pub async fn rebase_abort(dir: &Path, timeout_ms: u64) {
    match run_git(dir, &["rebase", "--abort"], timeout_ms).await {
        Ok(out) if out.exit_code != 0 => {
            tracing::debug!(stderr = %out.stderr.trim(), "rebase abort skipped");
        }
        Err(e) => tracing::debug!(error = %e, "rebase abort failed"),
        _ => {}
    }
}

pub async fn reset_hard(dir: &Path, timeout_ms: u64) -> Result<(), WError> {
    run_git_ok(dir, &["reset", "--hard"], timeout_ms)
        .await
        .map(|_| ())
}

/// Atomic fast-forward via compare-and-swap on the ref. `Ok(false)` means
/// the CAS lost (the target moved); the caller loops back to rebase.
pub async fn cas_update_ref(
    repo: &Path,
    target_branch: &str,
    new_sha: &str,
    old_sha: &str,
    timeout_ms: u64,
) -> Result<bool, WError> {
    let full = format!("refs/heads/{target_branch}");
    let out = run_git(repo, &["update-ref", &full, new_sha, old_sha], timeout_ms).await?;
    if out.exit_code == 0 {
        return Ok(true);
    }
    let stderr = out.stderr.trim();
    // A lost CAS (the target moved) is the one business outcome; git renders
    // it as `... is at <x> but expected <y>`. Lock contention, IO, or an
    // invalid ref are infra failures that must propagate, not silently retry.
    if stderr.contains("but expected") {
        tracing::debug!(target_branch, stderr, "update-ref CAS lost");
        return Ok(false);
    }
    Err(WError::new(
        codes::GIT_NONZERO,
        format!("update-ref {full} failed in {}: {stderr}", repo.display()),
    ))
}

/// Fast-forward merge inside a live checkout. `Ok(false)` when not
/// fast-forwardable (the target moved past the recorded base).
pub async fn merge_ff_only(dir: &Path, new_sha: &str, timeout_ms: u64) -> Result<bool, WError> {
    let out = run_git(dir, &["merge", "--ff-only", new_sha], timeout_ms).await?;
    Ok(out.exit_code == 0)
}

/// Find the checkout (primary or another worktree) that has `branch`
/// checked out, if any.
pub async fn find_checkout_of_branch(
    repo: &Path,
    branch: &str,
    timeout_ms: u64,
) -> Result<Option<WorktreeListEntry>, WError> {
    let full = format!("refs/heads/{branch}");
    let entries = worktree_list(repo, timeout_ms).await?;
    Ok(entries
        .into_iter()
        .find(|e| e.branch.as_deref() == Some(&full)))
}

/// `(behind, ahead)` of HEAD relative to `base`.
pub async fn ahead_behind(dir: &Path, base: &str, timeout_ms: u64) -> Result<(u64, u64), WError> {
    let spec = format!("{base}...HEAD");
    let out = run_git_ok(
        dir,
        &["rev-list", "--left-right", "--count", &spec],
        timeout_ms,
    )
    .await?;
    let text = out.stdout_trimmed();
    let mut parts = text.split_whitespace();
    let behind = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let ahead = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Ok((behind, ahead))
}

/// Human-readable `git diff --shortstat` of committed work since `base`.
pub async fn diffstat(dir: &Path, base: &str, timeout_ms: u64) -> Result<String, WError> {
    let spec = format!("{base}..HEAD");
    let out = run_git_ok(dir, &["diff", "--shortstat", &spec], timeout_ms).await?;
    Ok(out.stdout_trimmed())
}
