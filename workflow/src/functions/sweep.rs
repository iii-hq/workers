//! `workflow::sweep` — cron-bound node-timeout sweep.
//!
//! Scans all `AwaitingNodes` runs. For each, re-runs reconciliation (to pick up
//! any newly-completed nodes) and times out any `Running` checkpoint past its
//! `pending_at + pending_timeout_ms` deadline. Then re-enqueues a tick so the
//! run can advance.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{error::WorkflowError, functions::Deps, reconcile, state, types::NodeState};

use super::start;

pub const SWEEP_ID: &str = "workflow::sweep";
pub const SWEEP_DESC: &str =
    "Internal cron sweep: reconcile AwaitingNodes runs and time out nodes past their deadline. \
     Not called directly.";

/// Cron event payload (schedule info ignored — the sweep scans all run records).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct SweepEvent {
    #[serde(default)]
    pub scheduled_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SweepResponse {
    pub ok: bool,
    /// Number of runs touched (reconciled and/or timed out) this sweep.
    pub swept: u64,
}

/// How long a terminal run's records are retained before the sweep GCs them.
/// `workflow::status` / `workflow::node-result` return null for a run after this
/// window. ponytail: fixed 24h; lift to a WorkerConfig knob if operators ever need
/// to tune run-history retention.
const TERMINAL_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;

pub async fn handle(deps: &Deps, _event: SweepEvent) -> Result<SweepResponse, WorkflowError> {
    let runs = state::list_runs(&deps.iii).await?;
    let active = runs.iter().filter(|r| !r.status.is_terminal()).count() as u64;
    crate::telemetry::set_active_runs(active);
    let mut swept = 0u64;

    let cfg = deps.cfg().await;
    let now = deps.now_ms();

    for run in runs.iter().filter(|r| !r.status.is_terminal()) {
        // Isolate each run: one failing run must not starve the rest of the
        // cycle of timeout handling. Log and continue on error.
        match sweep_one_run(deps, &run.run_id, &cfg, now).await {
            Ok(true) => swept += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(run_id = %run.run_id, error = %e, "sweep: run failed; skipping")
            }
        }
    }

    // GC terminal runs past the retention window so `list_runs` doesn't deserialize
    // unbounded history every cycle (terminal runs are otherwise never deleted). Uses
    // the snapshot taken above, so a run that finalized THIS cycle (updated_at ~= now)
    // is never reaped here. Best-effort; a failed delete is retried next sweep.
    let cutoff = now - TERMINAL_RETENTION_MS;
    for run in runs
        .iter()
        .filter(|r| r.status.is_terminal() && r.updated_at < cutoff)
    {
        match state::delete_run(&deps.iii, run).await {
            Ok(()) => swept += 1,
            Err(e) => {
                tracing::warn!(run_id = %run.run_id, error = %e, "sweep: GC delete failed")
            }
        }
    }

    Ok(SweepResponse { ok: true, swept })
}

/// Reconcile + timeout-sweep a single run. Returns `Ok(true)` if the run was
/// touched (reconciled and/or timed out), `Ok(false)` if it was skipped
/// (vanished / orphaned / already terminal under the lock).
async fn sweep_one_run(
    deps: &Deps,
    run_id: &str,
    cfg: &crate::config::WorkerConfig,
    now: i64,
) -> Result<bool, WorkflowError> {
    // Acquire per-run lock to serialize against concurrent tick deliveries.
    let _g = deps.locks.guard(run_id).await;

    // Re-read after acquiring the lock — state may have changed.
    let Some(mut record) = state::get_run(&deps.iii, run_id).await? else {
        return Ok(false);
    };
    if record.status.is_terminal() {
        return Ok(false);
    }

    // Fetch the workflow definition — skip if missing (orphaned record).
    let def = match state::get_def(&deps.iii, &record.run_id).await? {
        Some(d) => d,
        None => return Ok(false),
    };

    // Poll running nodes for completion.
    reconcile::reconcile_run(deps, &mut record).await?;

    // Timeout sweep: apply timeout_action to each Running checkpoint.
    let results = state::load_done_results(&deps.iii, &mut record).await?;
    let default_timeout_ms = cfg.default_pending_timeout_ms;
    let max_retries = cfg.max_node_retries;
    let dispatch_timeout_ms = cfg.dispatch_timeout_ms;
    let mut timed_out_any = false;
    let node_uids: Vec<String> = record.nodes.keys().cloned().collect();
    for uid in node_uids {
        let cp = record.nodes.get(&uid).cloned().unwrap();
        match crate::timeout::timeout_action(&cp, default_timeout_ms, max_retries, now) {
            crate::timeout::TimeoutAction::StillWaiting => {}
            crate::timeout::TimeoutAction::Refire { attempt } => {
                // Stop the timed-out attempt's session before firing a fresh one,
                // otherwise the old harness turn keeps running (zombie / token burn).
                if let Some(sid) = cp.session_id.clone() {
                    super::harness_stop_session(deps, &sid, dispatch_timeout_ms).await;
                }
                if let Some(c) = record.nodes.get_mut(&uid) {
                    c.retries = attempt;
                }
                crate::telemetry::record_timeout(true);
                crate::functions::tick::fire_node(deps, &mut record, &def, &uid, &results).await?;
                timed_out_any = true;
            }
            crate::timeout::TimeoutAction::FailOut => {
                // Stop the timed-out session so it doesn't keep running after fail-out.
                if let Some(sid) = cp.session_id.clone() {
                    super::harness_stop_session(deps, &sid, dispatch_timeout_ms).await;
                }
                if let Some(c) = record.nodes.get_mut(&uid) {
                    c.state = NodeState::Failed;
                    c.result_error = Some("node_timeout".to_string());
                }
                crate::telemetry::record_timeout(false);
                // FailOut is terminal for this node, but reconcile only polls
                // Running nodes — emit the node-duration here too, else timed-out
                // (the slowest) nodes would be absent from the histogram.
                let dur = cp
                    .pending_at
                    .map(|p| (now - p).max(0) as f64)
                    .unwrap_or(0.0);
                crate::telemetry::record_node_terminal(false, dur);
                timed_out_any = true;
            }
        }
    }

    state::put_run(&deps.iii, &record).await?;

    // Re-drive: enqueue the next tick so the run can advance past the
    // newly-resolved nodes.
    if let Err(e) = start::enqueue_tick(&deps.iii, &record.run_id, record.step + 1).await {
        tracing::warn!(run_id = %record.run_id, error = %e, "sweep: re-enqueue tick failed");
    }

    if timed_out_any {
        tracing::info!(
            run_id = %record.run_id,
            "sweep: timed out one or more Running nodes"
        );
    }

    Ok(true)
}
