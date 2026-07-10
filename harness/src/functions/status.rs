//! `harness::status` — read the current turn status for a session
//! (harness.md § `harness::status`). Read-only; safe to expose to agents.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::turn::TurnStatus;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StatusRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChildRef {
    pub function_call_id: String,
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusReport {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub status: TurnStatus,
    pub step: u64,
    pub turn_count: u32,
    pub max_turns: u32,
    pub validation_retries: u32,
    pub max_validation_retries: u32,
    pub transient_resumes: u32,
    pub max_transient_resumes: u32,
    pub partial_result_available: bool,
    pub depth: u32,
    pub pending_function_calls: Vec<String>,
    pub children: Vec<ChildRef>,
    /// Messages queued while a step streams, in arrival order; they land in
    /// the transcript when the stream ends.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queued: Vec<crate::state::QueuedMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
}

/// `null` for unknown sessions.
pub async fn handle(deps: &Deps, req: StatusRequest) -> Result<Option<StatusReport>, HarnessError> {
    let cfg = deps.cfg().await;
    let Some(record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(None);
    };
    let queued =
        crate::state::list_queued(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    let children = record
        .live_children()
        .into_iter()
        .map(|c| ChildRef {
            function_call_id: c.function_call_id,
            session_id: c.session_id,
            turn_id: c.turn_id,
        })
        .collect();
    Ok(Some(StatusReport {
        session_id: record.session_id.clone(),
        turn_id: Some(record.turn_id.clone()),
        status: record.status,
        step: record.step,
        turn_count: record.turn_count,
        max_turns: record.options.max_turns,
        validation_retries: record.validation_retries,
        max_validation_retries: record.options.max_validation_retries,
        transient_resumes: record.transient_resumes,
        max_transient_resumes: record.options.max_transient_resumes,
        partial_result_available: record.result.is_some(),
        depth: record.depth,
        pending_function_calls: record.pending_call_ids(),
        children,
        queued,
        result: record.result.clone(),
        result_error: record.result_error.clone(),
    }))
}
