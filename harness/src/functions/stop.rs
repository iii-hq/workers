//! `harness::stop` — request cancellation of an in-flight turn (harness.md §
//! `harness::stop`). Sets the abort flag the next step observes and aborts an
//! in-flight stream via `router::abort`. The cascade to spawned children
//! layers on with sub-agents.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StopRequest {
    pub session_id: String,
    /// Omit to stop the current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StopResponse {
    pub stopping: bool,
}

pub async fn handle(deps: &Deps, req: StopRequest) -> Result<StopResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let Some(mut record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(StopResponse { stopping: false });
    };
    if let Some(tid) = &req.turn_id {
        if &record.turn_id != tid {
            return Ok(StopResponse { stopping: false });
        }
    }
    if record.status.is_terminal() {
        return Ok(StopResponse { stopping: false });
    }

    // Cascade to non-terminal spawned children first (harness.md § Cancellation
    // cascade): each child stop resolves the child's parent call with an error
    // when the child finalises.
    for child in record.live_children() {
        Box::pin(handle(
            deps,
            StopRequest {
                session_id: child.session_id.clone(),
                turn_id: Some(child.turn_id.clone()),
            },
        ))
        .await
        .ok();
    }

    record.abort = true;
    record.updated_at = crate::types::message::AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;

    // Abort an in-flight stream so the running step finalises promptly.
    if let Some(request_id) = &record.stream_request_id {
        let router = deps.router().await;
        router.abort(request_id).await;
    }

    Ok(StopResponse { stopping: true })
}
