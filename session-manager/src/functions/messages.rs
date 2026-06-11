//! `session::messages` — load the active path, oldest first.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{AgentMessage, CustomPayload, Role};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessagesRequest {
    pub session_id: String,
    /// Page size. Default 50, clamped to the configured maximum.
    pub limit: Option<usize>,
    /// Opaque pagination cursor from a previous response.
    pub cursor: Option<String>,
    /// Only messages with these roles. Setting this also excludes
    /// `kind: "custom"` entries (it is an explicit narrowing to roles).
    pub roles: Option<Vec<Role>>,
    /// Treat this entry as the leaf: return its parent chain,
    /// root -> entry, oldest first (branch view).
    pub from_entry_id: Option<String>,
    /// Interleave `kind: "custom"` entries at their path position.
    /// Default false.
    pub include_custom: Option<bool>,
}

/// One item of the active path: exactly one of `message` / `custom`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessageItem {
    pub entry_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomPayload>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessagesResponse {
    pub messages: Vec<MessageItem>,
    /// Present when more pages remain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub async fn handle(deps: &Deps, req: MessagesRequest) -> Result<MessagesResponse, SessionError> {
    let (resp, _) = deps.service.messages(req).await?;
    Ok(resp)
}
