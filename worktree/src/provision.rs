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
use std::sync::OnceLock;

use crate::config::{glob_match, ProvisionConfig};
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

/// Platform copy mode, probed once per process.
pub fn platform_copy_mode() -> CopyMode {
    static MODE: OnceLock<CopyMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        if cfg!(target_os = "macos") {
            CopyMode::MacClone
        } else if cfg!(target_os = "linux") {
            CopyMode::LinuxReflink
        } else {
            CopyMode::Plain
        }
    })
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
/// worktree nested under an ignored dir is never copied).
pub fn filter_entries(
    entries: Vec<String>,
    cfg: &ProvisionConfig,
    repo: &Path,
    worktree_root: &Path,
    registered_worktrees: &[String],
) -> Vec<String> {
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
            let absolute = repo.join(rel);
            if absolute.starts_with(worktree_root) {
                return false;
            }
            let abs_str = absolute.to_string_lossy();
            if registered_worktrees
                .iter()
                .any(|wt| wt.starts_with(abs_str.as_ref()) || abs_str.starts_with(wt.as_str()))
            {
                return false;
            }
            if !cfg.include.is_empty() && !cfg.include.iter().any(|g| glob_match(g, rel)) {
                return false;
            }
            if cfg.exclude.iter().any(|g| glob_match(g, rel)) {
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

async fn run_cp(program: &str, args: &[&str]) -> bool {
    let out = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    matches!(out, Ok(o) if o.status.success())
}

/// Copy one entry with the mode's cheapest flags, degrading to a plain
/// `cp -R` when the clone-capable invocation fails (non-APFS volume, older
/// coreutils). `program` is injectable for tests.
pub async fn copy_entry_with(program: &str, mode: CopyMode, src: &Path, dst: &Path) -> bool {
    let src_s = src.to_string_lossy();
    let dst_s = dst.to_string_lossy();
    let first: &[&str] = match mode {
        CopyMode::MacClone => &["-c", "-R", &src_s, &dst_s],
        CopyMode::LinuxReflink => &["-R", "--reflink=auto", &src_s, &dst_s],
        CopyMode::Plain => &["-R", &src_s, &dst_s],
    };
    if run_cp(program, first).await {
        return true;
    }
    if mode == CopyMode::Plain {
        return false;
    }
    run_cp(program, &["-R", &src_s, &dst_s]).await
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct CopySummary {
    pub copied: usize,
    pub bytes: u64,
    pub over_budget: bool,
}

/// Replicate the filtered gitignored entries from `repo` into `worktree`,
/// bounded by `max_copy_bytes`. Entries already present in the worktree are
/// left alone (idempotent under retries).
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

    let cp = "cp";
    let mut summary = CopySummary::default();
    for entry in entries {
        let rel = entry.trim_end_matches('/');
        let src = repo.join(rel);
        let dst = worktree.join(rel);
        if !src.exists() || dst.exists() {
            continue;
        }
        let size = entry_size(&src, cfg.max_copy_bytes);
        if summary.bytes.saturating_add(size) > cfg.max_copy_bytes {
            summary.over_budget = true;
            tracing::warn!(
                entry = rel,
                max_copy_bytes = cfg.max_copy_bytes,
                "provision copy budget exceeded; skipping the remaining entries"
            );
            break;
        }
        if let Some(parent) = dst.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                continue;
            }
        }
        if copy_entry_with(cp, mode, &src, &dst).await {
            summary.copied += 1;
            summary.bytes = summary.bytes.saturating_add(size);
        } else {
            tracing::warn!(entry = rel, "provision copy entry failed");
        }
    }
    Ok(summary)
}

/// Detached best-effort provisioning task, spawned after the create
/// response is already on the wire.
#[allow(clippy::too_many_arguments)]
pub fn spawn_copy_ignored(
    repo: PathBuf,
    worktree: PathBuf,
    cfg: ProvisionConfig,
    worktree_root: PathBuf,
    timeout_ms: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let registered = match crate::git::ops::worktree_list(&repo, timeout_ms).await {
            Ok(entries) => entries.into_iter().map(|e| e.path).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        match copy_ignored(
            &repo,
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
