//! Durable record shapes and shared wire types. Git owns existence; the
//! state store owns intent and ownership. Records without directories are
//! `orphaned`; directories without records are unmanaged and never adopted.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Lifecycle of a managed worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    /// Exists and is unclaimed.
    Active,
    /// Claimed by a session.
    Claimed,
    /// A land job is queued or running.
    Landing,
    /// The last land attempt was blocked (conflict, failed tests, ...).
    LandBlocked,
    /// The record exists but the directory is gone.
    Orphaned,
}

/// The durable record for one managed worktree (state scope `worktree`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeRecord {
    /// Stable id, `wt_<8hex>`. Doubles as the git admin dir name.
    pub worktree_id: String,
    /// The repository path the worktree was created from.
    pub repo_path: String,
    /// Canonicalized `git rev-parse --git-common-dir` — the per-repo serialization key.
    pub repo_key: String,
    /// Absolute path of the worktree directory.
    pub path: String,
    /// The branch checked out in the worktree.
    pub branch: String,
    /// The ref the worktree was created from.
    pub base_ref: String,
    /// The commit `base_ref` resolved to at creation time.
    pub base_sha: String,
    /// Current lifecycle.
    pub lifecycle: Lifecycle,
    /// Owning session, when claimed.
    pub session_id: Option<String>,
    /// Creation time, ms since epoch.
    pub created_at: i64,
    /// Last mutation time, ms since epoch.
    pub updated_at: i64,
}

/// Git status summary for one worktree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeStatus {
    /// True when there are no staged, unstaged, untracked, or conflicted entries.
    pub clean: bool,
    /// Commits ahead of the comparison base (upstream when set, else `base_sha`).
    pub ahead: u64,
    /// Commits behind the comparison base.
    pub behind: u64,
    /// Staged entry count.
    pub staged: u64,
    /// Unstaged entry count.
    pub unstaged: u64,
    /// Untracked entry count.
    pub untracked: u64,
    /// Unmerged (conflicted) entry count.
    pub conflicted: u64,
    /// `git diff --shortstat` of committed work since `base_sha`.
    pub diffstat: String,
    /// Commits not merged upstream (upstream ahead count, else commits past `base_sha`).
    pub unpushed: u64,
    /// True while a rebase is in progress in this worktree.
    pub in_rebase: bool,
    /// Current HEAD commit.
    pub head_sha: String,
}

/// One worktree as reported by `worktree::list` / `worktree::get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeInfo {
    /// Stable id, `wt_<8hex>`.
    pub worktree_id: String,
    /// The repository path the worktree was created from.
    pub repo_path: String,
    /// Canonicalized per-repo key.
    pub repo_key: String,
    /// Absolute path of the worktree directory.
    pub path: String,
    /// The branch checked out in the worktree.
    pub branch: String,
    /// The ref the worktree was created from.
    pub base_ref: String,
    /// The commit `base_ref` resolved to at creation time.
    pub base_sha: String,
    /// Current lifecycle (reported `orphaned` when the directory is gone).
    pub lifecycle: Lifecycle,
    /// Owning session, when claimed.
    pub session_id: Option<String>,
    /// Creation time, ms since epoch.
    pub created_at: i64,
    /// Last mutation time, ms since epoch.
    pub updated_at: i64,
    /// Git status, when requested and computable.
    pub status: Option<WorktreeStatus>,
}

impl WorktreeInfo {
    pub fn from_record(record: &WorktreeRecord, lifecycle: Lifecycle) -> Self {
        Self {
            worktree_id: record.worktree_id.clone(),
            repo_path: record.repo_path.clone(),
            repo_key: record.repo_key.clone(),
            path: record.path.clone(),
            branch: record.branch.clone(),
            base_ref: record.base_ref.clone(),
            base_sha: record.base_sha.clone(),
            lifecycle,
            session_id: record.session_id.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
            status: None,
        }
    }
}

/// Phase of a land job. Every phase is persisted before it executes and is
/// re-entrant under queue redelivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LandPhase {
    Preflight,
    Rebase,
    Test,
    Merge,
    Finalize,
    Done,
}

impl LandPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            LandPhase::Preflight => "preflight",
            LandPhase::Rebase => "rebase",
            LandPhase::Test => "test",
            LandPhase::Merge => "merge",
            LandPhase::Finalize => "finalize",
            LandPhase::Done => "done",
        }
    }
}

/// The durable land-job record (state scope `worktree_land_job`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LandJob {
    /// Stable id, `job_<8hex>`.
    pub job_id: String,
    /// The worktree being landed.
    pub worktree_id: String,
    /// Per-repo FIFO group key (canonical git common dir).
    pub repo_key: String,
    /// Repository path (primary checkout).
    pub repo_path: String,
    /// Absolute worktree directory.
    pub worktree_path: String,
    /// The branch being landed.
    pub branch: String,
    /// The branch to fast-forward.
    pub target_branch: String,
    /// Optional test command run in the worktree via `shell::exec`.
    pub test_cmd: Option<String>,
    /// Abort any in-progress rebase and reset before starting.
    pub force_restart: bool,
    /// Keep the worktree and branch after a successful land.
    pub keep: bool,
    /// Current phase; persisted before execution.
    pub phase: LandPhase,
    /// Compare-and-swap attempts consumed by target movement.
    pub cas_attempts: u32,
    /// The target commit recorded at preflight (the CAS `old` value).
    pub recorded_target_sha: Option<String>,
    /// Worktree HEAD after the successful rebase (the CAS `new` value).
    pub rebased_sha: Option<String>,
    /// The commit the test gate last passed for.
    pub test_passed_sha: Option<String>,
    /// The commit the target branch was advanced to.
    pub merged_sha: Option<String>,
    /// True when the job reached a terminal state (landed or blocked).
    pub done: bool,
    /// The `W###` code when the job ended blocked.
    pub blocked_code: Option<String>,
    /// Creation time, ms since epoch.
    pub created_at: i64,
    /// Last mutation time, ms since epoch.
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_value(Lifecycle::LandBlocked).unwrap(),
            serde_json::json!("land-blocked")
        );
    }

    #[test]
    fn land_phase_round_trips() {
        for p in [
            LandPhase::Preflight,
            LandPhase::Rebase,
            LandPhase::Test,
            LandPhase::Merge,
            LandPhase::Finalize,
            LandPhase::Done,
        ] {
            let v = serde_json::to_value(p).unwrap();
            let back: LandPhase = serde_json::from_value(v).unwrap();
            assert_eq!(back, p);
            assert_eq!(serde_json::to_value(p).unwrap(), p.as_str());
        }
    }
}
