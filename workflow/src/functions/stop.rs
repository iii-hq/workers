use crate::error::WorkflowError;
use crate::functions::{start, Deps};
use crate::types::RunStatus;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StopRequest {
    pub run_id: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StopResponse {
    pub stopping: bool,
}

// ---------------------------------------------------------------------------
// should_stop (pure classifier)
// ---------------------------------------------------------------------------

pub fn should_stop(status: RunStatus) -> bool {
    !status.is_terminal()
}

// ---------------------------------------------------------------------------
// handle
// ---------------------------------------------------------------------------

pub async fn handle(deps: &Deps, req: StopRequest) -> Result<StopResponse, WorkflowError> {
    let _g = deps.locks.guard(&req.run_id).await;

    let Some(mut record) = crate::state::get_run(&deps.iii, &req.run_id).await? else {
        return Ok(StopResponse { stopping: false }); // unknown run: no-op
    };

    if !should_stop(record.status) {
        return Ok(StopResponse { stopping: false }); // already terminal: no-op
    }

    // 1. Mark aborted, persist, enqueue a finalizing tick.
    record.abort = true;
    record.updated_at = deps.now_ms();
    crate::state::put_run(&deps.iii, &record).await?;
    start::enqueue_tick(&deps.iii, &req.run_id, record.step + 1).await?;

    // 2. Cascade: stop every node's live session (shared with finalize).
    let dispatch_timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    super::cascade_stop_running(deps, &record, dispatch_timeout_ms).await;

    Ok(StopResponse { stopping: true })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::types::RunStatus;

    #[test]
    fn stopping_false_for_terminal_status() {
        assert!(!super::should_stop(RunStatus::Completed));
        assert!(!super::should_stop(RunStatus::Cancelled));
        assert!(super::should_stop(RunStatus::Running));
        assert!(super::should_stop(RunStatus::AwaitingNodes));
    }
}
