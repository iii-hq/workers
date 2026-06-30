//! `workflow::wake` — fast-wake a run when one of its nodes finishes.
//!
//! Bound to `harness::turn-completed` (no session filter, so we receive every
//! terminal turn in the engine and discard non-workflow ones). A finished node
//! re-ticks its run within milliseconds instead of waiting for the next cron
//! sweep. Best-effort: events are at-least-once / unordered / lost if the worker
//! is down — the cron sweep + tick reconcile remain the durable wake path.

use serde::{Deserialize, Serialize};

use crate::error::WorkflowError;
use crate::functions::{start, Deps};

pub const WAKE_ID: &str = "workflow::wake";
pub const WAKE_DESC: &str =
    "Internal: on a harness turn-completed event, wake the owning run with a tick. \
     Best-effort latency optimization; the cron sweep is the durable backstop. \
     Not called directly.";

/// Lenient view of the `harness::turn-completed` payload — we only need the
/// session id. The event also carries turn_id/status/result/timestamp/parent,
/// which we ignore (no `deny_unknown_fields`).
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct WakeEvent {
    #[serde(default)]
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct WakeResponse {
    pub woke: bool,
}

/// Node sessions are deterministic `wf_<run_id>_<node_uid>@r<n>` ids. The reverse
/// index resolves run_id; this prefix check cheaply rejects every non-workflow
/// turn before any state I/O.
pub fn is_workflow_session(session_id: &str) -> bool {
    session_id.starts_with("wf_")
}

pub async fn handle(deps: &Deps, ev: WakeEvent) -> Result<WakeResponse, WorkflowError> {
    if !is_workflow_session(&ev.session_id) {
        return Ok(WakeResponse { woke: false });
    }

    let Some(run_id) = crate::state::run_id_for_session(&deps.iii, &ev.session_id).await? else {
        return Ok(WakeResponse { woke: false }); // not one of ours / unindexed
    };

    let Some(record) = crate::state::get_run(&deps.iii, &run_id).await? else {
        return Ok(WakeResponse { woke: false }); // run gone
    };
    if record.status.is_terminal() {
        return Ok(WakeResponse { woke: false }); // already done
    }

    // Wake: a tick reconciles the just-finished node and advances the run. The
    // step guard + deterministic sids make a duplicate/out-of-order event a no-op.
    start::enqueue_tick(&deps.iii, &run_id, record.step + 1).await?;
    Ok(WakeResponse { woke: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wf_prefixed_sessions_are_workflow_sessions() {
        assert!(is_workflow_session("wf_run_abc_plan@r0"));
        assert!(is_workflow_session("wf_run_abc_read#3@r1"));
        assert!(!is_workflow_session("sess_other_worker"));
        assert!(!is_workflow_session("")); // empty/missing session id
        assert!(!is_workflow_session("workflow_run")); // not the wf_ session prefix
    }
}
