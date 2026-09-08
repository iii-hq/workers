//! `session::messages-range` — the whole entries behind a collapsed run.
//!
//! `session::messages-tail` hands out activity runs with their heavy parts
//! left out; this is how a reader gets them back, either a whole span
//! ("show all" on one group) or a handful of specific entries (calls a
//! renderer must draw even while collapsed). Full content, always; nothing
//! is ever elided here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::messages_tail::TailItem;
use super::Deps;
use crate::error::SessionError;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessagesRangeRequest {
    pub session_id: String,
    /// With `to_entry_id`: every entry of the active path from this one to
    /// `to_entry_id`, both inclusive, oldest first. Exactly one selector
    /// (`from_entry_id`+`to_entry_id`, or `entry_ids`) is required.
    pub from_entry_id: Option<String>,
    pub to_entry_id: Option<String>,
    /// Specific entries, returned in path order (duplicates folded).
    pub entry_ids: Option<Vec<String>>,
    /// Page size in entries. Default 50, clamped to the configured maximum;
    /// a long run comes back in several pages.
    pub limit: Option<usize>,
    /// Opaque pagination cursor from a previous response.
    pub cursor: Option<String>,
    /// Default true. When false, `image` blocks that carry an
    /// `attachment_id` come back with `data: ""` (as `session::messages`).
    pub include_image_data: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessagesRangeResponse {
    /// Oldest first; never `elided`.
    pub messages: Vec<TailItem>,
    /// Present when more entries of the selection remain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub async fn handle(
    deps: &Deps,
    req: MessagesRangeRequest,
) -> Result<MessagesRangeResponse, SessionError> {
    deps.service.messages_range(req).await
}
