//! `worktree::status` — git status for one managed worktree, plus the
//! shared status builder used by `get` and `list`.

use std::collections::HashMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::WError;
use crate::functions::create::{require_dir, require_record};
use crate::functions::Deps;
use crate::git::ops;
use crate::types::{Lifecycle, WorktreeRecord, WorktreeStatus};

/// Handler-local memo of resolved integration targets, keyed by
/// `(repo_path, base_ref)`. Lives for one handler invocation only, so a
/// list over many worktrees of one repo resolves the target once.
pub type TargetCache = HashMap<(String, String), Option<(String, String)>>;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to inspect.
    pub worktree_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The inspected worktree.
    pub worktree_id: String,
    /// The branch checked out in the worktree.
    pub branch: String,
    /// Current lifecycle.
    pub lifecycle: Lifecycle,
    /// Advisory dev-server port derived from the worktree id.
    pub dev_port: u16,
    /// Git status details.
    #[serde(flatten)]
    pub status: WorktreeStatus,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let record = require_record(deps, &req.worktree_id).await?;
    require_dir(&record)?;
    let status = build_status(&record, cfg.git_timeout_ms, &mut TargetCache::new()).await?;
    Ok(Response {
        dev_port: crate::ids::dev_port(&record.worktree_id),
        worktree_id: record.worktree_id,
        branch: record.branch,
        lifecycle: record.lifecycle,
        status,
    })
}

/// Compute the full status summary for one worktree. Ahead/behind and
/// `unpushed` come from the upstream when one is set, else from `base_sha`.
pub async fn build_status(
    record: &WorktreeRecord,
    git_timeout_ms: u64,
    targets: &mut TargetCache,
) -> Result<WorktreeStatus, WError> {
    let wt = Path::new(&record.path);
    let t = git_timeout_ms;
    let st = ops::status(wt, t).await?;
    let in_rebase = ops::is_rebase_in_progress(wt, t).await?;
    let head_sha = match st.oid.clone() {
        Some(oid) => oid,
        None => ops::rev_parse(wt, "HEAD", t).await?,
    };
    let (behind_base, ahead_base) = ops::ahead_behind(wt, &record.base_sha, t).await?;
    let (ahead, behind, unpushed) = if st.has_upstream {
        (st.ahead, st.behind, st.ahead)
    } else {
        (ahead_base, behind_base, ahead_base)
    };
    let diffstat = ops::diffstat(wt, &record.base_sha, t).await?;
    let integration = integration_of(record, &head_sha, t, targets).await;
    Ok(WorktreeStatus {
        clean: st.clean(),
        ahead,
        behind,
        staged: st.staged,
        unstaged: st.unstaged,
        untracked: st.untracked,
        conflicted: st.conflicted,
        diffstat,
        unpushed,
        in_rebase,
        head_sha,
        integrated: integration.integrated,
        integration_reason: integration.reason.map(str::to_string),
    })
}

/// Best-effort integration probe; a failed check degrades to not-integrated
/// rather than failing the whole status read. Target resolution is memoized
/// in `targets` for the duration of one handler invocation.
pub async fn integration_of(
    record: &WorktreeRecord,
    head_sha: &str,
    git_timeout_ms: u64,
    targets: &mut TargetCache,
) -> ops::Integration {
    let wt = Path::new(&record.path);
    let repo = Path::new(&record.repo_path);
    let key = (record.repo_path.clone(), record.base_ref.clone());
    if !targets.contains_key(&key) {
        let resolved = match ops::integration_target(repo, &record.base_ref, git_timeout_ms).await {
            Ok(resolved) => resolved,
            Err(e) => {
                tracing::debug!(error = %e, "integration target resolution failed");
                None
            }
        };
        targets.insert(key.clone(), resolved);
    }
    let Some((target_name, target_sha)) = targets.get(&key).and_then(Clone::clone) else {
        return ops::Integration::no();
    };
    if target_name == record.branch {
        return ops::Integration::no();
    }
    match ops::check_integration(wt, head_sha, &target_sha, git_timeout_ms).await {
        Ok(integration) => integration,
        Err(e) => {
            tracing::debug!(error = %e, "integration check failed");
            ops::Integration::no()
        }
    }
}
