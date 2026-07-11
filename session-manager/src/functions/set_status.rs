//! `session::set-status` — set the session lifecycle status.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::SessionStatus;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetStatusRequest {
    pub session_id: String,
    /// Target status: `idle` / `working` / `done` / `error`.
    pub status: SessionStatus,
    /// Short cause/phase stored as `status_reason` — kept on `error`
    /// (failure cause) and `working` (live phase detail), cleared on
    /// `idle`/`done`.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SetStatusResponse {
    pub status: SessionStatus,
    pub previous_status: SessionStatus,
}

pub async fn handle(deps: &Deps, req: SetStatusRequest) -> Result<SetStatusResponse, SessionError> {
    let (resp, events) = deps.service.set_status(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
