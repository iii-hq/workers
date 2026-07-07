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
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &spec,
        ],
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
    Ok(crate::git::canonical_or_self(&absolute)
        .to_string_lossy()
        .into_owned())
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
        &["show-ref", "--verify", "--quiet", "--end-of-options", &full],
        timeout_ms,
    )
    .await?;
    Ok(out.exit_code == 0)
}

/// Best-effort branch delete; `-d` unless `force`. A missing branch is a no-op.
pub async fn branch_delete(repo: &Path, branch: &str, force: bool, timeout_ms: u64) -> bool {
    let flag = if force { "-D" } else { "-d" };
    match run_git(
        repo,
        &["branch", flag, "--end-of-options", branch],
        timeout_ms,
    )
    .await
    {
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
    let out = run_git(
        repo,
        &["update-ref", "--end-of-options", &full, new_sha, old_sha],
        timeout_ms,
    )
    .await?;
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

/// CAS branch delete: remove `refs/heads/<branch>` only while it still
/// points at `expected_sha`. `Ok(false)` when the branch moved (or is
/// already gone) — the branch is retained rather than dropped blind.
pub async fn cas_branch_delete(
    repo: &Path,
    branch: &str,
    expected_sha: &str,
    timeout_ms: u64,
) -> Result<bool, WError> {
    let full = format!("refs/heads/{branch}");
    let out = run_git(
        repo,
        &["update-ref", "-d", "--end-of-options", &full, expected_sha],
        timeout_ms,
    )
    .await?;
    if out.exit_code != 0 {
        tracing::debug!(branch, stderr = %out.stderr.trim(), "CAS branch delete refused");
        return Ok(false);
    }
    Ok(true)
}

/// Data-safety anchor for a land in flight: `refs/iii/wt-backup/<id>`
/// pins the branch head from before the rebase until finalize succeeds.
pub fn backup_ref(worktree_id: &str) -> String {
    format!("refs/iii/wt-backup/{worktree_id}")
}

pub async fn set_backup_ref(
    repo: &Path,
    worktree_id: &str,
    sha: &str,
    timeout_ms: u64,
) -> Result<(), WError> {
    let full = backup_ref(worktree_id);
    run_git_ok(
        repo,
        &["update-ref", "--end-of-options", &full, sha],
        timeout_ms,
    )
    .await
    .map(|_| ())
}

/// Best-effort backup-ref cleanup after a successful land.
pub async fn delete_backup_ref(repo: &Path, worktree_id: &str, timeout_ms: u64) {
    let full = backup_ref(worktree_id);
    match run_git(
        repo,
        &["update-ref", "-d", "--end-of-options", &full],
        timeout_ms,
    )
    .await
    {
        Ok(out) if out.exit_code != 0 => {
            tracing::debug!(stderr = %out.stderr.trim(), "backup ref delete skipped");
        }
        Err(e) => tracing::debug!(error = %e, "backup ref delete failed"),
        _ => {}
    }
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
        &[
            "rev-list",
            "--left-right",
            "--count",
            "--end-of-options",
            &spec,
        ],
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

/// Fetch a GitHub pull request head from `origin` into `refs/iii/pr/<n>`
/// and return the commit it points at. `W112` with a clean message when
/// origin publishes no such ref (non-GitHub layout, wrong number).
pub async fn fetch_pr_head(repo: &Path, pr: u64, timeout_ms: u64) -> Result<String, WError> {
    let refspec = format!("+refs/pull/{pr}/head:refs/iii/pr/{pr}");
    // The runner's GIT_PROTOCOL_FROM_USER=0 also blocks file:// remotes;
    // re-allow them here only. The remote URL is operator configuration
    // (origin), never caller input — the PR number is the only variable.
    let out = run_git(
        repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "fetch",
            "--no-tags",
            "origin",
            &refspec,
        ],
        timeout_ms,
    )
    .await?;
    if out.exit_code != 0 {
        return Err(WError::new(
            codes::REF_NOT_FOUND,
            format!(
                "refs/pull/{pr}/head not found on origin (GitHub pull request layout \
                 required): {}",
                out.stderr.trim()
            ),
        ));
    }
    rev_parse(repo, &format!("refs/iii/pr/{pr}"), timeout_ms).await
}

// ---------------------------------------------------------------------------
// Integration detection
// ---------------------------------------------------------------------------

/// Cap on the target-branch commit walk during the patch-id fallback. A
/// squash merge lands as one target commit, so scanning the most recent
/// window is enough; beyond it the check conservatively reports
/// not-integrated instead of walking unbounded history.
pub const PATCH_ID_WALK_CAP: u64 = 500;

/// Outcome of the integration check chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Integration {
    pub integrated: bool,
    /// Which check matched: `same_commit`, `ancestor`, `no_added_changes`,
    /// `trees_match`, `merge_adds_nothing`, or `patch_id_match`.
    pub reason: Option<&'static str>,
}

impl Integration {
    pub(crate) fn no() -> Self {
        Self {
            integrated: false,
            reason: None,
        }
    }

    pub(crate) fn yes(reason: &'static str) -> Self {
        Self {
            integrated: true,
            reason: Some(reason),
        }
    }
}

/// The branch a worktree's work is expected to merge into: its recorded
/// base ref when that is a real ref, else the primary checkout's current
/// branch. Returns `(name, sha)`; `None` when neither resolves (detached
/// primary, deleted base).
pub async fn integration_target(
    repo: &Path,
    base_ref: &str,
    timeout_ms: u64,
) -> Result<Option<(String, String)>, WError> {
    let name = if base_ref != "HEAD" {
        base_ref.to_string()
    } else {
        let out = run_git(repo, &["symbolic-ref", "--short", "HEAD"], timeout_ms).await?;
        if out.exit_code != 0 {
            return Ok(None);
        }
        let name = out.stdout_trimmed();
        if name.is_empty() {
            return Ok(None);
        }
        name
    };
    match rev_parse(repo, &name, timeout_ms).await {
        Ok(sha) => Ok(Some((name, sha))),
        Err(e) if e.code == codes::REF_NOT_FOUND => Ok(None),
        Err(e) => Err(e),
    }
}

/// Both commits' tree ids in one spawn (one output line per commit). No
/// `--end-of-options` here: in list mode rev-parse echoes it into stdout,
/// and both arguments are engine-resolved commit shas, never caller input.
async fn trees_of(
    dir: &Path,
    a: &str,
    b: &str,
    timeout_ms: u64,
) -> Result<(String, String), WError> {
    let spec_a = format!("{a}^{{tree}}");
    let spec_b = format!("{b}^{{tree}}");
    let out = run_git_ok(dir, &["rev-parse", &spec_a, &spec_b], timeout_ms).await?;
    let mut lines = out.stdout.lines();
    let tree_a = lines.next().unwrap_or("").trim().to_string();
    let tree_b = lines.next().unwrap_or("").trim().to_string();
    Ok((tree_a, tree_b))
}

/// Cheapest-first integration chain: is every change on `branch_sha` already
/// contained in the commit `target_sha`? Catches fast-forwards and true
/// merges (ancestor), rebases (matching trees), squash merges (merge adds
/// nothing, or the patch-id fallback when the squashed files were touched
/// again), and no-op branches. A diverged branch with real unmerged work
/// reports not-integrated. Runs inside the worktree; objects are shared
/// repo-wide.
pub async fn check_integration(
    dir: &Path,
    branch_sha: &str,
    target_sha: &str,
    timeout_ms: u64,
) -> Result<Integration, WError> {
    if branch_sha == target_sha {
        return Ok(Integration::yes("same_commit"));
    }

    // One merge-base spawn answers both questions: unrelated histories are
    // never integrated, and a merge base equal to the branch head means the
    // branch is an ancestor of the target.
    let based = run_git(
        dir,
        &["merge-base", "--end-of-options", branch_sha, target_sha],
        timeout_ms,
    )
    .await?;
    if based.exit_code != 0 {
        return Ok(Integration::no());
    }
    let merge_base = based.stdout_trimmed();
    if merge_base == branch_sha {
        return Ok(Integration::yes("ancestor"));
    }

    let added = run_git_ok(
        dir,
        &[
            "diff",
            "--name-only",
            &format!("{merge_base}..{branch_sha}"),
        ],
        timeout_ms,
    )
    .await?;
    if added.stdout.trim().is_empty() {
        return Ok(Integration::yes("no_added_changes"));
    }

    let (branch_tree, target_tree) = trees_of(dir, branch_sha, target_sha, timeout_ms).await?;
    if branch_tree == target_tree {
        return Ok(Integration::yes("trees_match"));
    }

    // In-memory merge: exit 0 = clean (first line is the resulting tree);
    // exit 1 = conflict (fall through to patch-ids); anything else (e.g.
    // git without the merge-tree write-tree plumbing) also falls through.
    let merged = run_git(
        dir,
        &["merge-tree", "--write-tree", target_sha, branch_sha],
        timeout_ms,
    )
    .await?;
    if merged.exit_code == 0 {
        let merged_tree = merged.stdout.lines().next().unwrap_or("").trim();
        if merged_tree == target_tree {
            return Ok(Integration::yes("merge_adds_nothing"));
        }
        return Ok(Integration::no());
    }

    patch_id_match(dir, branch_sha, target_sha, &merge_base, timeout_ms).await
}

/// Squash-merge fallback: the branch's combined diff (merge base to head)
/// carries the same patch-id as some commit already on the target. Uses
/// diff-tree plumbing so user diff config never skews the ids, and caps the
/// target walk at [`PATCH_ID_WALK_CAP`].
async fn patch_id_match(
    dir: &Path,
    branch_sha: &str,
    target_sha: &str,
    merge_base: &str,
    timeout_ms: u64,
) -> Result<Integration, WError> {
    // One walk, capped at one past the limit: line count doubles as the
    // commit count, and the same stdout feeds `diff-tree --stdin` below.
    let max_count = format!("--max-count={}", PATCH_ID_WALK_CAP + 1);
    let target_commits = run_git_ok(
        dir,
        &[
            "rev-list",
            &max_count,
            "--end-of-options",
            &format!("{merge_base}..{target_sha}"),
        ],
        timeout_ms,
    )
    .await?;
    let count = target_commits.stdout.lines().count() as u64;
    if count == 0 || count > PATCH_ID_WALK_CAP {
        return Ok(Integration::no());
    }

    let branch_diff = run_git_ok(
        dir,
        &["diff-tree", "-p", merge_base, branch_sha],
        timeout_ms,
    )
    .await?;
    let branch_patch = crate::git::run_git_with_stdin(
        dir,
        &["patch-id", "--verbatim"],
        branch_diff.stdout.as_bytes(),
        timeout_ms,
    )
    .await?;
    let Some(branch_id) = branch_patch.stdout.split_whitespace().next() else {
        return Ok(Integration::no());
    };
    let branch_id = branch_id.to_string();

    let target_diffs = crate::git::run_git_with_stdin(
        dir,
        &["diff-tree", "--stdin", "-p"],
        target_commits.stdout.as_bytes(),
        timeout_ms,
    )
    .await?;
    let target_ids = crate::git::run_git_with_stdin(
        dir,
        &["patch-id", "--verbatim"],
        target_diffs.stdout.as_bytes(),
        timeout_ms,
    )
    .await?;
    let matched = target_ids
        .stdout
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .any(|id| id == branch_id);
    Ok(if matched {
        Integration::yes("patch_id_match")
    } else {
        Integration::no()
    })
}
