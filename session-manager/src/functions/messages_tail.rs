//! `session::messages-tail` — the newest page of the active path, walking
//! backwards in block-aligned pages with activity runs collapsed.
//!
//! This is the reader a chat UI opens a session with: a few exchanges, the
//! rest behind an upward scroll. `session::messages` keeps its
//! oldest-first contract untouched; see [`crate::pagination`] for why the
//! two are separate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{AgentMessage, CustomPayload};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessagesTailRequest {
    pub session_id: String,
    /// Page size in *blocks* (an entry, or one whole activity run), not
    /// entries: a page is "N conversational steps", and a tool run of 300
    /// calls counts once. Default 50, clamped to the configured maximum.
    pub limit: Option<usize>,
    /// Exclusive upper bound: return the page ending just before the block
    /// that contains this entry (the oldest entry the caller already has).
    /// Omitted = the newest page. `session/invalid_cursor` when the entry is
    /// not on the active path (e.g. after a fork moved the leaf), so callers
    /// reload from the top.
    pub before_entry_id: Option<String>,
    /// Extend the page backwards until the block containing this entry is
    /// included, ignoring `limit` (deep links: "go to message"). No-op when
    /// the entry is already within the page or newer than it.
    /// `session/invalid_cursor` when not on the active path.
    pub until_entry_id: Option<String>,
    /// Interleave `kind: "custom"` entries at their path position.
    /// Default TRUE here (unlike `session::messages`): a transcript reader
    /// wants compaction markers and wake records in place.
    pub include_custom: Option<bool>,
    /// Default true. When false, `image` blocks that carry an
    /// `attachment_id` come back with `data: ""` (as `session::messages`).
    pub include_image_data: Option<bool>,
}

/// One item of the page: exactly one of `message` / `custom`. Same fields as
/// `session::messages` items plus `elided`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TailItem {
    pub entry_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomPayload>,
    /// The entry's writer-supplied origin, as on `session::messages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::types::JsonMap>,
    /// Present and true when this entry sits inside a collapsed activity
    /// run and the heavy parts of its message were left out: `function_call`
    /// blocks keep `id` + `function_id` but carry `arguments: {}`,
    /// `function_result` messages keep `function_call_id` + `is_error` with
    /// `content: []` and `details: null`, thinking is dropped, text is kept.
    /// `session::messages-range` returns the whole entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elided: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessagesTailResponse {
    /// Oldest first, like `session::messages`.
    pub messages: Vec<TailItem>,
    /// True when older blocks exist before this page.
    pub has_more: bool,
    /// The first entry of this page: pass it as `before_entry_id` to fetch
    /// the page before. Absent on an empty page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_entry_id: Option<String>,
}

pub async fn handle(
    deps: &Deps,
    req: MessagesTailRequest,
) -> Result<MessagesTailResponse, SessionError> {
    deps.service.messages_tail(req).await
}
