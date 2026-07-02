//! Async worktree removal: rename the directory into
//! `<worktree_root>/.trash/<worktree_id>-<epoch-secs>` (instant on the same
//! filesystem), return immediately, and let a detached task do the real
//! delete. The cron prune sweep clears trash entries older than 24 hours as
//! the backstop for deletes interrupted by a crash or restart.

use std::path::{Path, PathBuf};

use crate::error::{codes, WError};
use crate::types::now_ms;

pub const TRASH_DIR_NAME: &str = ".trash";

/// Backstop age before the prune sweep clears a trash entry whose detached
/// delete never finished.
pub const TRASH_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

/// How long the best-effort busy probe may hold up a removal.
const BUSY_PROBE_TIMEOUT_MS: u64 = 2_000;

pub fn trash_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(TRASH_DIR_NAME)
}

/// `<worktree_id>-<epoch-secs>`; the suffix drives the sweep's age check.
fn trash_entry_name(worktree_id: &str, now: i64) -> String {
    format!("{worktree_id}-{}", now / 1000)
}

/// Rename the worktree directory into the trash. Same-filesystem rename by
/// construction (the trash lives under the same worktree root), so this is
/// a metadata operation; the caller falls back to a direct removal when it
/// fails (permissions, exotic mounts).
pub fn stage(
    worktree_path: &Path,
    worktree_root: &Path,
    worktree_id: &str,
) -> Result<PathBuf, WError> {
    let dir = trash_dir(worktree_root);
    std::fs::create_dir_all(&dir).map_err(|e| {
        WError::new(
            codes::BAD_PATH,
            format!("create trash dir {}: {e}", dir.display()),
        )
    })?;
    let staged = dir.join(trash_entry_name(worktree_id, now_ms()));
    std::fs::rename(worktree_path, &staged).map_err(|e| {
        WError::new(
            codes::BAD_PATH,
            format!("stage {} into trash: {e}", worktree_path.display()),
        )
    })?;
    Ok(staged)
}

/// Detached delete of a staged trash entry. Failures are logged; the sweep
/// retries anything left behind.
pub fn spawn_delete(staged: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = tokio::fs::remove_dir_all(&staged).await {
            tracing::warn!(
                path = %staged.display(),
                error = %e,
                "trash delete failed; the prune sweep will retry"
            );
        }
    })
}

/// Parse the `-<epoch-secs>` suffix of a trash entry name. Entries without
/// one were not created by this worker and are never touched.
pub fn parse_trash_timestamp(name: &str) -> Option<i64> {
    let (_, ts) = name.rsplit_once('-')?;
    ts.parse::<i64>().ok().map(|secs| secs * 1000)
}

/// Trash entries older than [`TRASH_MAX_AGE_MS`] as of `now`.
pub fn stale_entries(worktree_root: &Path, now: i64) -> Vec<PathBuf> {
    let dir = trash_dir(worktree_root);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let staged_at = parse_trash_timestamp(&name)?;
            (now.saturating_sub(staged_at) >= TRASH_MAX_AGE_MS).then(|| entry.path())
        })
        .collect()
}

/// Remove stale trash entries; returns how many were cleared.
pub async fn sweep(worktree_root: &Path, now: i64) -> usize {
    let mut cleared = 0;
    for path in stale_entries(worktree_root, now) {
        match tokio::fs::remove_dir_all(&path).await {
            Ok(()) => cleared += 1,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "trash sweep entry failed")
            }
        }
    }
    cleared
}

/// Best-effort busy probe: does any running process hold something open
/// under `dir`? Uses `lsof -t +D` with a short timeout; `None` when the
/// probe is unavailable (no lsof, timeout) so removal proceeds. Never
/// signals or kills anything.
pub async fn dir_in_use(dir: &Path) -> Option<bool> {
    let mut cmd = tokio::process::Command::new("lsof");
    cmd.arg("-t")
        .arg("+D")
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(
        std::time::Duration::from_millis(BUSY_PROBE_TIMEOUT_MS),
        cmd.output(),
    )
    .await
    .ok()?
    .ok()?;
    // lsof exits 0 with pids on matches, 1 with empty output on none.
    Some(!out.stdout.trim_ascii().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_entry_names_carry_epoch_seconds() {
        let name = trash_entry_name("wt_ab12cd34", 1_700_000_123_456);
        assert_eq!(name, "wt_ab12cd34-1700000123");
        assert_eq!(parse_trash_timestamp(&name), Some(1_700_000_123_000));
    }

    #[test]
    fn unparseable_entries_are_ignored() {
        assert_eq!(parse_trash_timestamp("no-suffix-here"), None);
        assert_eq!(parse_trash_timestamp("plain"), None);
    }

    #[test]
    fn stale_entries_honor_the_age_threshold() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dir = trash_dir(root);
        std::fs::create_dir_all(&dir).unwrap();
        let now = now_ms();
        let fresh = dir.join(trash_entry_name("wt_fresh000", now));
        let stale = dir.join(trash_entry_name(
            "wt_stale000",
            now - TRASH_MAX_AGE_MS - 60_000,
        ));
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::create_dir_all(dir.join("manual-junk")).unwrap();

        let found = stale_entries(root, now);
        assert_eq!(found, vec![stale]);
    }
}
