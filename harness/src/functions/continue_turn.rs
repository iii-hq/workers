//! `harness::continue` — resume a turn parked at `max_turns` (the "ask to
//! continue" pause). The explicit counterpart to replying via `harness::send`:
//! a UI can wire a "Continue" button to this without composing a message, which
//! matters because a parked turn is non-terminal and the chat input is blocked.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ContinueRequest {
    pub session_id: String,
    /// Omit to continue the session's current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContinueResponse {
    /// True when a parked turn was resumed; false when there was nothing to
    /// continue (no turn, wrong turn, terminal, or not awaiting continuation).
    pub continuing: bool,
}

/// Resume a parked-at-`max_turns` turn with a fresh budget. No-op (returns
/// `continuing: false`) when the turn is missing, terminal, or not awaiting
/// continuation, so a double-click or stale button is harmless.
pub async fn handle(deps: &Deps, req: ContinueRequest) -> Result<ContinueResponse, HarnessError> {
    let cfg = deps.cfg().await;
    // Serialize against the loop / resolve / sends so the step and budget can't
    // be double-bumped.
    let _guard = deps.locks.guard(&req.session_id).await;
    let Some(mut record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(ContinueResponse { continuing: false });
    };
    if let Some(tid) = &req.turn_id {
        if &record.turn_id != tid {
            return Ok(ContinueResponse { continuing: false });
        }
    }
    if record.status.is_terminal() || !record.awaiting_continue {
        return Ok(ContinueResponse { continuing: false });
    }
    crate::functions::send::resume_awaiting_continue(deps, &cfg, &mut record).await?;
    Ok(ContinueResponse { continuing: true })
}
