//! Copy-ignored provisioning: after `worktree::create` returns, a spawned
//! background task replicates the source repo's gitignored files (.env,
//! caches, local settings) into the fresh worktree. Best-effort by design:
//! failures log and never affect the create; nothing new is emitted.
//!
//! Enumeration collapses fully-ignored directories (`git ls-files
//! --directory`), filters through the operator's include/exclude globs,
//! always drops VCS metadata dirs and anything under the managed worktree
//! root, and copies with the platform's cheapest mechanism: APFS clones on
//! macOS (`cp -c`, plain fallback), `--reflink=auto` on Linux.

use std::path::{Path, PathBuf};

use crate::config::{compile_globs, ProvisionConfig};
use crate::error::WError;
use crate::git::run_git_ok;

/// Directory components never copied, whatever the globs say.
const VCS_DIRS: [&str; 5] = [".git", ".bzr", ".hg", ".jj", ".svn"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// macOS `cp -c` (APFS clonefile), plain fallback per entry.
    MacClone,
    /// Linux `cp -R --reflink=auto` (degrades to a plain copy by itself).
    LinuxReflink,
    /// Plain `cp -R`.
    Plain,
}

/// Platform copy mode: a compile-time choice, no runtime probing.
pub fn platform_copy_mode() -> CopyMode {
    if cfg!(target_os = "macos") {
        CopyMode::MacClone
    } else if cfg!(target_os = "linux") {
        CopyMode::LinuxReflink
    } else {
        CopyMode::Plain
    }
}

/// Repo-relative gitignored entries, fully-ignored directories collapsed
/// to one `dir/` entry.
pub async fn enumerate_ignored(repo: &Path, timeout_ms: u64) -> Result<Vec<String>, WError> {
    let out = run_git_ok(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ],
        timeout_ms,
    )
    .await?;
    Ok(out
        .stdout
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect())
}

/// Apply include/exclude globs plus the always-on exclusions: VCS metadata
/// dirs, the managed worktree root, and registered worktree paths (a
/// worktree nested under an ignored dir is never copied). `repo_canon` must
/// be the canonicalized repository path: git reports canonical worktree
/// paths, and a symlinked spelling here (macOS /var vs /private/var) would
/// silently disable the nested-worktree exclusion.
pub fn filter_entries(
    entries: Vec<String>,
    cfg: &ProvisionConfig,
    repo_canon: &Path,
    worktree_root: &Path,
    registered_worktrees: &[String],
) -> Vec<String> {
    let include = compile_globs(&cfg.include);
    let exclude = compile_globs(&cfg.exclude);
    let worktree_root = crate::git::canonical_or_self(worktree_root);
    entries
        .into_iter()
        .filter(|entry| {
            let rel = entry.trim_end_matches('/');
            if rel.is_empty() {
                return false;
            }
            if Path::new(rel)
                .components()
                .any(|c| VCS_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
            {
                return false;
            }
            let absolute = repo_canon.join(rel);
            if absolute.starts_with(&worktree_root) {
                return false;
            }
            // Component-wise on both sides so `/repo/node_modules2` never
            // reads as nested under a registered `/repo/node_modules`.
            if registered_worktrees.iter().any(|wt| {
                let wt = Path::new(wt);
                wt.starts_with(&absolute) || absolute.starts_with(wt)
            }) {
                return false;
            }
            if !cfg.include.is_empty() && !include.is_match(rel) {
                return false;
            }
            if exclude.is_match(rel) {
                return false;
            }
            true
        })
        .collect()
}

/// Recursive size of an entry, stopping early once `budget` is exceeded.
fn entry_size(path: &Path, budget: u64) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return total;
    };
    for entry in entries.filter_map(Result::ok) {
        total = total.saturating_add(entry_size(&entry.path(), budget));
        if total > budget {
            return total;
        }
    }
    total
}

fn run_cp(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Copy one entry with the mode's cheapest flags, degrading to a plain
/// `cp -R` when the clone-capable invocation fails (non-APFS volume, older
/// coreutils). Blocking by design: it runs on the copy loop's blocking
/// thread. `program` is injectable for tests.
pub fn copy_entry_with(program: &str, mode: CopyMode, src: &Path, dst: &Path) -> bool {
    let src_s = src.to_string_lossy();
    let dst_s = dst.to_string_lossy();
    let first: &[&str] = match mode {
        CopyMode::MacClone => &["-c", "-R", &src_s, &dst_s],
        CopyMode::LinuxReflink => &["-R", "--reflink=auto", &src_s, &dst_s],
        CopyMode::Plain => &["-R", &src_s, &dst_s],
    };
    if run_cp(program, first) {
        return true;
    }
    if mode == CopyMode::Plain {
        return false;
    }
    run_cp(program, &["-R", &src_s, &dst_s])
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct CopySummary {
    pub copied: usize,
    pub bytes: u64,
    pub over_budget: bool,
}

/// Replicate the filtered gitignored entries from `repo` (canonicalized by
/// the caller) into `worktree`, bounded by `max_copy_bytes`. Entries already
/// present in the worktree are left alone (idempotent under retries). The
/// size scans and copies are blocking filesystem work and run on a blocking
/// thread, off the async runtime.
pub async fn copy_ignored(
    repo: &Path,
    worktree: &Path,
    cfg: &ProvisionConfig,
    registered_worktrees: &[String],
    worktree_root: &Path,
    mode: CopyMode,
    timeout_ms: u64,
) -> Result<CopySummary, WError> {
    let raw = enumerate_ignored(repo, timeout_ms).await?;
    let entries = filter_entries(raw, cfg, repo, worktree_root, registered_worktrees);

    let repo = repo.to_path_buf();
    let worktree = worktree.to_path_buf();
    let max_copy_bytes = cfg.max_copy_bytes;
    let summary = tokio::task::spawn_blocking(move || {
        let mut summary = CopySummary::default();
        for entry in entries {
            // Liveness guard: `worktree::remove` renames the directory away
            // instantly (trash staging), so a vanished root means the
            // worktree is being torn down and copying on would recreate its
            // parents. The record lives in async state and this loop is
            // blocking, so the cheap per-entry probe is the right signal.
            if !worktree.is_dir() {
                tracing::debug!(
                    worktree = %worktree.display(),
                    "worktree gone mid-provision; aborting the remaining entries"
                );
                break;
            }
            let rel = entry.trim_end_matches('/');
            let src = repo.join(rel);
            let dst = worktree.join(rel);
            if !src.exists() || dst.exists() {
                continue;
            }
            let size = entry_size(&src, max_copy_bytes);
            if summary.bytes.saturating_add(size) > max_copy_bytes {
                summary.over_budget = true;
                tracing::warn!(
                    entry = rel,
                    max_copy_bytes,
                    "provision copy budget exceeded; skipping the remaining entries"
                );
                break;
            }
            if let Some(parent) = dst.parent() {
                if std::fs::create_dir_all(parent).is_err() {
                    continue;
                }
            }
            if copy_entry_with("cp", mode, &src, &dst) {
                summary.copied += 1;
                summary.bytes = summary.bytes.saturating_add(size);
            } else {
                tracing::warn!(entry = rel, "provision copy entry failed");
            }
        }
        summary
    })
    .await
    .unwrap_or_else(|e| {
        // Best-effort contract: a lost copy task degrades to an empty
        // summary rather than failing the caller.
        tracing::warn!(error = %e, "provision copy task did not finish");
        CopySummary::default()
    });
    Ok(summary)
}

/// Detached best-effort provisioning task, spawned after the create
/// response is already on the wire.
pub fn spawn_copy_ignored(
    repo: PathBuf,
    worktree: PathBuf,
    cfg: ProvisionConfig,
    worktree_root: PathBuf,
    timeout_ms: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // The listing includes the primary checkout itself; excluding it
        // would exclude every repository entry, so drop it (canonical
        // compare: git reports canonical paths, callers may not).
        let repo_canon = crate::git::canonical_or_self(&repo);
        let registered = match crate::git::ops::worktree_list(&repo, timeout_ms).await {
            Ok(entries) => entries
                .into_iter()
                .map(|e| e.path)
                .filter(|p| Path::new(p) != repo_canon)
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        match copy_ignored(
            &repo_canon,
            &worktree,
            &cfg,
            &registered,
            &worktree_root,
            platform_copy_mode(),
            timeout_ms,
        )
        .await
        {
            Ok(summary) => tracing::info!(
                copied = summary.copied,
                bytes = summary.bytes,
                over_budget = summary.over_budget,
                worktree = %worktree.display(),
                "provisioned gitignored files"
            ),
            Err(e) => {
                tracing::warn!(error = %e, worktree = %worktree.display(), "provisioning failed")
            }
        }
    })
}
