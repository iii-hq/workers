//! Function registration. One file per function; every request and response
//! is a typed struct deriving `JsonSchema` (the registry publish gate rejects
//! untyped schemas). Registration order is pinned by `surface::catalog()` and
//! the golden schema tests.

pub mod claim;
pub mod create;
pub mod get;
pub mod land;
pub mod land_step;
pub mod list;
pub mod prune;
pub mod release;
pub mod remove;
pub mod status;
pub mod validate;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::events::Emitter;
use crate::land::test_gate::TestRunner;
use crate::land::{Enqueuer, LandDeps};
use crate::locks::RepoLocks;
use crate::state::StateStore;

/// Shared handler dependencies. Everything effectful is behind a trait so
/// the whole surface is testable against real temp repos without an engine.
pub struct Deps {
    pub cell: ConfigCell,
    pub state: Arc<dyn StateStore>,
    pub locks: RepoLocks,
    pub emitter: Arc<Emitter>,
    pub enqueuer: Arc<dyn Enqueuer>,
    pub test_runner: Arc<dyn TestRunner>,
}

impl Deps {
    /// Cheap per-call snapshot of the live config.
    pub async fn cfg(&self) -> Arc<WorkerConfig> {
        self.cell.read().await.clone()
    }

    pub fn land_deps(&self, cfg: &WorkerConfig) -> LandDeps {
        LandDeps {
            state: self.state.clone(),
            locks: self.locks.clone(),
            emitter: self.emitter.clone(),
            test_runner: self.test_runner.clone(),
            git_timeout_ms: cfg.git_timeout_ms,
            test_timeout_ms: cfg.test_timeout_ms,
            max_land_retries: cfg.max_land_retries,
            allow_branch_delete: cfg.gates.allow_branch_delete,
        }
    }
}

/// `W500` denial naming the exact config key to flip.
pub(crate) fn gate_denied(op: &str, key: &str) -> crate::error::WError {
    crate::error::WError::new(
        crate::error::codes::OP_DISABLED,
        format!("{op} is disabled by configuration; set {key}: true to enable"),
    )
}

/// `W501` denial for the force paths, all behind one key.
pub(crate) fn force_denied(op: &str) -> crate::error::WError {
    crate::error::WError::new(
        crate::error::codes::FORCE_DISABLED,
        format!("{op} is disabled by configuration; set gates.allow_force: true to enable"),
    )
}

/// Allowlist denial (`W502`/`W503`) naming the rejected subject and the
/// glob-list config key to extend.
pub(crate) fn allowlist_denied(
    code: &'static str,
    subject: &str,
    key: &str,
) -> crate::error::WError {
    crate::error::WError::new(
        code,
        format!("{subject} is not allowed by configuration; add it to {key} to enable"),
    )
}

/// `W504` denial carrying the live count that hit the per-repo budget.
pub(crate) fn budget_denied(repo: &std::path::Path, live: u32) -> crate::error::WError {
    crate::error::WError::new(
        crate::error::codes::WORKTREE_BUDGET_EXCEEDED,
        format!(
            "repository {} already has {live} live worktree(s), the \
             configured budget; raise gates.max_worktrees_per_repo to \
             allow more",
            repo.display()
        ),
    )
}

macro_rules! register {
    ($iii:expr, $deps:expr, $id:expr, $module:ident, $desc:expr) => {{
        let deps = $deps.clone();
        $iii.register_function(
            $id,
            RegisterFunction::new_async(move |req: $module::Request| {
                let deps = deps.clone();
                async move { $module::handle(&deps, req).await.map_err(Error::from) }
            })
            .description($desc),
        );
    }};
}

pub fn register_all(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    register!(
        iii,
        deps,
        "worktree::create",
        create,
        "Create an isolated, locked git worktree off a base ref and register it \
         in the cross-agent registry. Returns the path to hand to an agent as \
         its working directory."
    );
    register!(
        iii,
        deps,
        "worktree::list",
        list,
        "List managed worktrees, optionally filtered by repo or session and \
         enriched with git status. Also reports unmanaged worktrees of the \
         repo when repo_path is given."
    );
    register!(
        iii,
        deps,
        "worktree::get",
        get,
        "Fetch one managed worktree with its current git status."
    );
    register!(
        iii,
        deps,
        "worktree::validate",
        validate,
        "Validate a path as a managed worktree (console picker surface). \
         Unmanaged worktrees are reported but never adopted."
    );
    register!(
        iii,
        deps,
        "worktree::claim",
        claim,
        "Claim a worktree for a session. Fails with W210 when another session \
         holds it, unless force is set."
    );
    register!(
        iii,
        deps,
        "worktree::release",
        release,
        "Release a session's claim. Fails with W211 when the caller does not \
         hold the claim, unless force is set."
    );
    register!(
        iii,
        deps,
        "worktree::status",
        status,
        "Git status for one worktree: cleanliness, ahead/behind, diffstat, \
         rebase-in-progress, lifecycle."
    );
    register!(
        iii,
        deps,
        "worktree::remove",
        remove,
        "Remove a managed worktree. Refuses dirty (W220) or unmerged (W221) \
         work unless force is set; optionally deletes the branch."
    );
    register!(
        iii,
        deps,
        "worktree::prune",
        prune,
        "Sweep managed worktrees: reconcile records whose directories are gone \
         and remove clean, unclaimed, expired worktrees. Cron-safe: an empty \
         payload uses defaults; dry_run reports without acting."
    );
    register!(
        iii,
        deps,
        "worktree::land",
        land,
        "Queue a land: rebase onto the target, run the optional test gate via \
         shell::exec, fast-forward the target with an atomic CAS, then clean \
         up. Serialized per repo via the land queue."
    );
    register!(
        iii,
        deps,
        "worktree::land-step",
        land_step,
        "Internal: execute one queued land job. Bound to the land queue; not \
         agent-callable."
    );
    tracing::info!("all worktree functions registered");
}
