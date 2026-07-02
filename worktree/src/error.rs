//! Structured error codes for the `worktree::*` surface, mirroring the
//! `shell` worker's `S###` convention. Every error a caller can receive
//! carries a stable `W###` prefix so agents and operators can branch on the
//! code instead of parsing prose.

use iii_sdk::errors::Error;

pub mod codes {
    /// Business validation failed after deserialize (empty field, bad combination).
    pub const INVALID_REQUEST: &str = "W001";
    /// The stored configuration could not be applied.
    pub const CONFIG_INVALID: &str = "W010";
    /// Spawning the `git` subprocess failed.
    pub const GIT_SPAWN: &str = "W100";
    /// The `git` subprocess exceeded its timeout.
    pub const GIT_TIMEOUT: &str = "W101";
    /// The `git` subprocess exited nonzero.
    pub const GIT_NONZERO: &str = "W102";
    /// The supplied path is not inside a git repository.
    pub const NOT_A_REPO: &str = "W110";
    /// The supplied path or ref-like value is malformed (relative, `-`-prefixed, non-UTF-8).
    pub const BAD_PATH: &str = "W111";
    /// The requested ref does not resolve to a commit.
    pub const REF_NOT_FOUND: &str = "W112";
    /// The requested branch already exists.
    pub const BRANCH_EXISTS: &str = "W120";
    /// No worktree record exists for the supplied id.
    pub const NOT_FOUND: &str = "W200";
    /// The record exists but its directory is gone (orphaned).
    pub const PATH_MISSING: &str = "W201";
    /// The worktree is claimed by another session.
    pub const ALREADY_CLAIMED: &str = "W210";
    /// The releasing session does not own the claim.
    pub const CLAIM_MISMATCH: &str = "W211";
    /// The worktree has uncommitted changes.
    pub const DIRTY: &str = "W220";
    /// The worktree has commits not merged anywhere.
    pub const UNMERGED_WORK: &str = "W221";
    /// The iii state store could not be reached (retryable).
    pub const STATE_UNAVAILABLE: &str = "W300";
    /// Enqueueing the land job failed.
    pub const ENQUEUE_FAILED: &str = "W400";
    /// A land job is already queued or running for this worktree.
    pub const LAND_IN_PROGRESS: &str = "W401";
    /// A previous land left a rebase in progress; resolve it or pass force_restart.
    pub const LAND_CONFLICTED: &str = "W402";
    /// The rebase onto the target hit conflicts (left in progress for in-place resolution).
    pub const REBASE_CONFLICT: &str = "W410";
    /// The test gate failed (nonzero exit, or the shell jail rejected the worktree path).
    pub const TEST_FAILED: &str = "W411";
    /// The target branch kept moving past max_land_retries.
    pub const TARGET_MOVED_EXHAUSTED: &str = "W412";
    /// The target branch is checked out in a dirty checkout.
    pub const TARGET_CHECKED_OUT_DIRTY: &str = "W413";
}

/// One structured worker error: stable code + human-readable message.
#[derive(Debug, Clone)]
pub struct WError {
    pub code: &'static str,
    pub message: String,
}

impl WError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Wire rendering: `"W102: git worktree add exited with 128: ..."`.
    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

impl std::fmt::Display for WError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.render())
    }
}

impl std::error::Error for WError {}

impl From<WError> for Error {
    fn from(e: WError) -> Self {
        Error::Handler(e.render())
    }
}

impl From<serde_json::Error> for WError {
    fn from(e: serde_json::Error) -> Self {
        WError::new(codes::STATE_UNAVAILABLE, format!("serde: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_prefixes_code() {
        let e = WError::new(codes::DIRTY, "worktree has uncommitted changes");
        assert_eq!(e.render(), "W220: worktree has uncommitted changes");
    }

    #[test]
    fn converts_to_handler_error() {
        let e: Error = WError::new(codes::NOT_FOUND, "no record").into();
        assert_eq!(e.to_string(), "handler error: W200: no record");
    }
}
