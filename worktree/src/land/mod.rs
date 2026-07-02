//! The land phase machine: preflight -> rebase -> test -> merge -> finalize.
//!
//! Correctness contract:
//! - Every phase is persisted to the durable job record BEFORE it executes,
//!   and every phase re-inspects real git state on entry, so queue
//!   redelivery at any point converges without duplicate side effects.
//! - Business failures (rebase conflict, failed tests, dirty tree, moved
//!   target, checked-out-dirty target) return `Ok` and emit a
//!   `worktree::land-blocked` event; only infra failures (git spawn/timeout,
//!   state store) return `Err` so the engine queue redelivers.
//! - The merge is a fast-forward compare-and-swap: `git update-ref
//!   refs/heads/<target> <new> <old>`. A lost CAS loops back to rebase,
//!   bounded by `max_land_retries`, inside the same invocation and under
//!   the same per-repo lock.
//! - When the target branch is checked out in a live checkout: fast-forward
//!   inside that checkout when clean; block with W413 when dirty (never
//!   desync a checkout's HEAD from its tree).

pub mod test_gate;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, TriggerAction};
use serde_json::Value;

use crate::error::{codes, WError};
use crate::events::{Emitter, EventCtx, EventKind, LandBlockedEvent, LandedEvent};
use crate::git::ops;
use crate::locks::RepoLocks;
use crate::state::{self, StateStore};
use crate::types::{now_ms, LandJob, LandPhase, Lifecycle, WorktreeRecord};

use test_gate::TestRunner;

/// Queue dispatch behind a trait so the land handler is testable in-process.
#[async_trait]
pub trait Enqueuer: Send + Sync {
    async fn enqueue(&self, queue: &str, function_id: &str, payload: Value) -> Result<(), WError>;
}

pub struct IiiEnqueuer {
    iii: Arc<IIIClient>,
}

impl IiiEnqueuer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl Enqueuer for IiiEnqueuer {
    async fn enqueue(&self, queue: &str, function_id: &str, payload: Value) -> Result<(), WError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(TriggerAction::Enqueue {
                    queue: queue.to_string(),
                }),
                timeout_ms: None,
            })
            .await
            .map(|_| ())
            .map_err(|e| WError::new(codes::ENQUEUE_FAILED, format!("enqueue to {queue:?}: {e}")))
    }
}

/// Everything one land step needs, injected so tests run with an in-memory
/// store and a scripted test runner against real temp repos.
pub struct LandDeps {
    pub state: Arc<dyn StateStore>,
    pub locks: RepoLocks,
    pub emitter: Arc<Emitter>,
    pub test_runner: Arc<dyn TestRunner>,
    pub git_timeout_ms: u64,
    pub test_timeout_ms: u64,
    pub max_land_retries: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub phase_completed: LandPhase,
    pub done: bool,
}

/// Drive the job from its persisted phase to a terminal state (landed or
/// blocked). Idempotent: a redelivery re-enters at the persisted phase and
/// converges.
pub async fn run_step(deps: &LandDeps, job_id: &str) -> Result<StepResult, WError> {
    let Some(mut job) = state::get_job(deps.state.as_ref(), job_id).await? else {
        // Unknown or already garbage-collected job: redelivery after
        // completion must be a no-op.
        return Ok(StepResult {
            phase_completed: LandPhase::Done,
            done: true,
        });
    };
    if job.done {
        return Ok(StepResult {
            phase_completed: job.phase,
            done: true,
        });
    }

    let _guard = deps.locks.guard(&job.repo_key).await;
    let git_t = deps.git_timeout_ms;

    loop {
        match job.phase {
            LandPhase::Preflight => {
                let wt = Path::new(&job.worktree_path);
                if !wt.is_dir() {
                    return block(
                        deps,
                        &mut job,
                        codes::PATH_MISSING,
                        "worktree_path_missing",
                        None,
                        None,
                    )
                    .await;
                }
                if job.force_restart {
                    if ops::is_rebase_in_progress(wt, git_t).await? {
                        ops::rebase_abort(wt, git_t).await;
                    }
                    ops::reset_hard(wt, git_t).await?;
                } else if ops::is_rebase_in_progress(wt, git_t).await? {
                    return block(
                        deps,
                        &mut job,
                        codes::LAND_CONFLICTED,
                        "rebase_in_progress",
                        None,
                        None,
                    )
                    .await;
                }
                let st = ops::status(wt, git_t).await?;
                if !st.clean() {
                    return block(deps, &mut job, codes::DIRTY, "dirty", None, None).await;
                }
                let repo = Path::new(&job.repo_path);
                match ops::rev_parse(repo, &job.target_branch, git_t).await {
                    Ok(sha) => job.recorded_target_sha = Some(sha),
                    Err(e) if e.code == codes::REF_NOT_FOUND => {
                        return block(
                            deps,
                            &mut job,
                            codes::REF_NOT_FOUND,
                            "target_ref_not_found",
                            None,
                            None,
                        )
                        .await;
                    }
                    Err(e) => return Err(e),
                }
                set_phase(deps, &mut job, LandPhase::Rebase).await?;
            }

            LandPhase::Rebase => {
                let wt = Path::new(&job.worktree_path);
                let in_rebase = ops::is_rebase_in_progress(wt, git_t).await?;
                if in_rebase {
                    let conflicts = ops::conflict_files(wt, git_t).await?;
                    if !conflicts.is_empty() {
                        return block(
                            deps,
                            &mut job,
                            codes::REBASE_CONFLICT,
                            "rebase_conflict",
                            Some(conflicts),
                            None,
                        )
                        .await;
                    }
                    // Crash mid-rebase with no conflicts: abort and retry cleanly.
                    ops::rebase_abort(wt, git_t).await;
                }
                let head = ops::rev_parse(wt, "HEAD", git_t).await?;
                if job.rebased_sha.as_deref() == Some(head.as_str()) {
                    set_phase(deps, &mut job, LandPhase::Test).await?;
                    continue;
                }
                let target = job.recorded_target_sha.clone().ok_or_else(|| {
                    WError::new(codes::STATE_UNAVAILABLE, "job missing recorded_target_sha")
                })?;
                let ok = ops::rebase_onto(wt, &target, git_t).await?;
                if !ok {
                    if ops::is_rebase_in_progress(wt, git_t).await? {
                        let conflicts = ops::conflict_files(wt, git_t).await?;
                        return block(
                            deps,
                            &mut job,
                            codes::REBASE_CONFLICT,
                            "rebase_conflict",
                            Some(conflicts),
                            None,
                        )
                        .await;
                    }
                    return block(
                        deps,
                        &mut job,
                        codes::REBASE_CONFLICT,
                        "rebase_failed",
                        None,
                        None,
                    )
                    .await;
                }
                job.rebased_sha = Some(ops::rev_parse(wt, "HEAD", git_t).await?);
                set_phase(deps, &mut job, LandPhase::Test).await?;
            }

            LandPhase::Test => {
                let Some(cmd) = job.test_cmd.clone() else {
                    set_phase(deps, &mut job, LandPhase::Merge).await?;
                    continue;
                };
                if job.test_passed_sha.is_some() && job.test_passed_sha == job.rebased_sha {
                    set_phase(deps, &mut job, LandPhase::Merge).await?;
                    continue;
                }
                let outcome = match deps
                    .test_runner
                    .run(&cmd, &job.worktree_path, deps.test_timeout_ms)
                    .await
                {
                    Ok(outcome) => outcome,
                    // A jail rejection is a configuration problem, not a
                    // transient fault: block instead of redelivering forever.
                    Err(e) if e.code == codes::TEST_FAILED => {
                        return block(
                            deps,
                            &mut job,
                            codes::TEST_FAILED,
                            "test_failed",
                            None,
                            None,
                        )
                        .await;
                    }
                    Err(e) => return Err(e),
                };
                if outcome.exit_code != 0 {
                    tracing::info!(
                        job_id = %job.job_id,
                        exit_code = outcome.exit_code,
                        tail = %outcome.tail,
                        "land test gate failed"
                    );
                    return block(
                        deps,
                        &mut job,
                        codes::TEST_FAILED,
                        "test_failed",
                        None,
                        Some(outcome.exit_code),
                    )
                    .await;
                }
                job.test_passed_sha = job.rebased_sha.clone();
                set_phase(deps, &mut job, LandPhase::Merge).await?;
            }

            LandPhase::Merge => {
                let repo = Path::new(&job.repo_path);
                let new = job.rebased_sha.clone().ok_or_else(|| {
                    WError::new(codes::STATE_UNAVAILABLE, "job missing rebased_sha")
                })?;
                let old = job.recorded_target_sha.clone().ok_or_else(|| {
                    WError::new(codes::STATE_UNAVAILABLE, "job missing recorded_target_sha")
                })?;

                // Redelivery after a successful CAS but before the phase
                // persist: the target already points at `new`.
                let current = ops::rev_parse(repo, &job.target_branch, git_t).await?;
                let merged = if current == new {
                    true
                } else if let Some(checkout) =
                    ops::find_checkout_of_branch(repo, &job.target_branch, git_t).await?
                {
                    let checkout_dir = Path::new(&checkout.path);
                    let st = ops::status(checkout_dir, git_t).await?;
                    if !st.clean() {
                        return block(
                            deps,
                            &mut job,
                            codes::TARGET_CHECKED_OUT_DIRTY,
                            "target_checked_out_dirty",
                            None,
                            None,
                        )
                        .await;
                    }
                    ops::merge_ff_only(checkout_dir, &new, git_t).await?
                } else {
                    ops::cas_update_ref(repo, &job.target_branch, &new, &old, git_t).await?
                };

                if merged {
                    job.merged_sha = Some(new);
                    set_phase(deps, &mut job, LandPhase::Finalize).await?;
                    continue;
                }

                // The target moved. Loop back to rebase, bounded.
                job.cas_attempts += 1;
                if job.cas_attempts > deps.max_land_retries {
                    return block(
                        deps,
                        &mut job,
                        codes::TARGET_MOVED_EXHAUSTED,
                        "target_moved_exhausted",
                        None,
                        None,
                    )
                    .await;
                }
                job.recorded_target_sha =
                    Some(ops::rev_parse(repo, &job.target_branch, git_t).await?);
                job.rebased_sha = None;
                job.test_passed_sha = None;
                set_phase(deps, &mut job, LandPhase::Rebase).await?;
            }

            LandPhase::Finalize => {
                let repo = Path::new(&job.repo_path);
                let wt = Path::new(&job.worktree_path);
                let merged_sha = job.merged_sha.clone().unwrap_or_default();
                let record = state::get_record(deps.state.as_ref(), &job.worktree_id).await?;
                let session_id = record.as_ref().and_then(|r| r.session_id.clone());

                if job.keep {
                    if let Some(mut record) = record {
                        record.lifecycle = Lifecycle::Active;
                        record.base_ref = job.target_branch.clone();
                        record.base_sha = merged_sha.clone();
                        record.updated_at = now_ms();
                        state::put_record(deps.state.as_ref(), &record).await?;
                    }
                } else {
                    if wt.exists() {
                        ops::worktree_unlock(repo, wt, git_t).await;
                        if let Err(e) = ops::worktree_remove(repo, wt, true, git_t).await {
                            tracing::warn!(error = %e, "finalize: worktree remove failed");
                        }
                    }
                    // `git branch -d` checks mergedness against HEAD, not the
                    // land target, so verify the ancestry explicitly and then
                    // force-delete: the target already contains every commit
                    // of the landed branch.
                    let target_ref = format!("refs/heads/{}", job.target_branch);
                    let branch_ref = format!("refs/heads/{}", job.branch);
                    match ops::is_ancestor(repo, &branch_ref, &target_ref, git_t).await {
                        Ok(true) => {
                            ops::branch_delete(repo, &job.branch, true, git_t).await;
                        }
                        Ok(false) => tracing::warn!(
                            branch = %job.branch,
                            "finalize: branch is not contained in the target; keeping it"
                        ),
                        Err(e) => {
                            tracing::debug!(error = %e, "finalize: ancestry check failed")
                        }
                    }
                    state::delete_record(deps.state.as_ref(), &job.worktree_id).await?;
                }

                state::clear_active_job_id(deps.state.as_ref(), &job.worktree_id).await?;
                deps.emitter
                    .emit(
                        EventKind::Landed,
                        EventCtx {
                            repo_path: &job.repo_path,
                            worktree_id: &job.worktree_id,
                            session_id: session_id.as_deref(),
                        },
                        &LandedEvent {
                            worktree_id: job.worktree_id.clone(),
                            repo_path: job.repo_path.clone(),
                            branch: job.branch.clone(),
                            target_branch: job.target_branch.clone(),
                            merged_sha,
                            session_id: session_id.clone(),
                            timestamp: now_ms(),
                        },
                    )
                    .await;

                job.phase = LandPhase::Done;
                job.done = true;
                job.updated_at = now_ms();
                state::put_job(deps.state.as_ref(), &job).await?;
                return Ok(StepResult {
                    phase_completed: LandPhase::Finalize,
                    done: true,
                });
            }

            LandPhase::Done => {
                return Ok(StepResult {
                    phase_completed: LandPhase::Done,
                    done: true,
                });
            }
        }
    }
}

/// Persist the next phase BEFORE executing it.
async fn set_phase(deps: &LandDeps, job: &mut LandJob, phase: LandPhase) -> Result<(), WError> {
    job.phase = phase;
    job.updated_at = now_ms();
    state::put_job(deps.state.as_ref(), job).await
}

/// Terminal business block: persist the blocked job, flip the record to
/// `land-blocked`, clear the active marker, and emit `worktree::land-blocked`.
/// Returns `Ok` — a blocked land is an outcome, not an infra failure.
async fn block(
    deps: &LandDeps,
    job: &mut LandJob,
    code: &'static str,
    reason: &str,
    conflict_files: Option<Vec<String>>,
    exit_code: Option<i32>,
) -> Result<StepResult, WError> {
    let phase = job.phase;
    job.blocked_code = Some(code.to_string());
    job.done = true;
    job.updated_at = now_ms();
    state::put_job(deps.state.as_ref(), job).await?;

    let mut session_id = None;
    if let Some(mut record) = state::get_record(deps.state.as_ref(), &job.worktree_id).await? {
        session_id = record.session_id.clone();
        record.lifecycle = Lifecycle::LandBlocked;
        record.updated_at = now_ms();
        state::put_record(deps.state.as_ref(), &record).await?;
    }
    state::clear_active_job_id(deps.state.as_ref(), &job.worktree_id).await?;

    tracing::info!(
        job_id = %job.job_id,
        worktree_id = %job.worktree_id,
        code,
        reason,
        "land blocked"
    );
    deps.emitter
        .emit(
            EventKind::LandBlocked,
            EventCtx {
                repo_path: &job.repo_path,
                worktree_id: &job.worktree_id,
                session_id: session_id.as_deref(),
            },
            &LandBlockedEvent {
                worktree_id: job.worktree_id.clone(),
                repo_path: job.repo_path.clone(),
                target_branch: job.target_branch.clone(),
                reason: reason.to_string(),
                code: code.to_string(),
                conflict_files,
                exit_code,
                session_id: session_id.clone(),
                timestamp: now_ms(),
            },
        )
        .await;

    Ok(StepResult {
        phase_completed: phase,
        done: true,
    })
}

/// Build a fresh job record for `worktree::land`.
#[allow(clippy::too_many_arguments)]
pub fn new_job(
    job_id: String,
    record: &WorktreeRecord,
    target_branch: String,
    test_cmd: Option<String>,
    force_restart: bool,
    keep: bool,
) -> LandJob {
    let now = now_ms();
    LandJob {
        job_id,
        worktree_id: record.worktree_id.clone(),
        repo_key: record.repo_key.clone(),
        repo_path: record.repo_path.clone(),
        worktree_path: record.path.clone(),
        branch: record.branch.clone(),
        target_branch,
        test_cmd,
        force_restart,
        keep,
        phase: LandPhase::Preflight,
        cas_attempts: 0,
        recorded_target_sha: None,
        rebased_sha: None,
        test_passed_sha: None,
        merged_sha: None,
        done: false,
        blocked_code: None,
        created_at: now,
        updated_at: now,
    }
}
