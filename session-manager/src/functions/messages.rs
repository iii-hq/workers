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
    /// Return only entries strictly after this entry id on the
    /// requested path (incremental fetch from a known watermark).
    /// Errors `session/invalid_cursor` when the entry is not on the
    /// path — e.g. after a fork moved the active leaf — so callers
    /// fall back to a full load. Ignored when `cursor` is set.
    pub after_entry_id: Option<String>,
    /// Only messages with these roles. Setting this also excludes
    /// `kind: "custom"` entries (it is an explicit narrowing to roles).
    pub roles: Option<Vec<Role>>,
    /// Treat this entry as the leaf: return its parent chain,
    /// root -> entry, oldest first (branch view).
    pub from_entry_id: Option<String>,
    /// Interleave `kind: "custom"` entries at their path position.
    /// Default false.
    pub include_custom: Option<bool>,
    /// Default true. When false, `image` blocks that carry an
    /// `attachment_id` come back with `data: ""`; the reader fetches the
    /// bytes on demand with `session::get-attachment`. Images without a
    /// stored original are returned inline regardless. Lets a UI open a
    /// long, picture-heavy transcript without pulling every image first;
    /// model-bound readers keep the default.
    pub include_image_data: Option<bool>,
    /// Default `"full"`. `"usage"` returns each message without the
    /// content a usage reader never looks at: assistant content keeps only
    /// its `function_call` blocks (with `arguments: null`); user, function
    /// result and custom-role content is empty and their `details` null;
    /// `origin` is left out. Role, usage, model/provider, stop reason,
    /// timestamps, `is_error` and `function_id` stay, so every message
    /// still parses as an `AgentMessage`. `kind: "custom"` entries
    /// (`include_custom`) come back as stored.
    pub content: Option<ContentProjection>,
}

/// How much of each message `session::messages` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentProjection {
    Full,
    Usage,
}

/// One item of the active path: exactly one of `message` / `custom`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessageItem {
    pub entry_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomPayload>,
    /// The entry's writer-supplied origin (turn correlation plus hook
    /// annotations, e.g. which memory bank and facts fed the generate).
    /// Absent on custom entries and on entries written without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::types::JsonMap>,
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
