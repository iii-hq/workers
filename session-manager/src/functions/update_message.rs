//! `session::update-message` — replace a message entry's content
//! (streaming deltas, edited function output).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Deps;
use crate::error::SessionError;
use crate::types::{ContentBlock, JsonMap, Usage};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateMessageRequest {
    pub session_id: String,
    pub entry_id: String,
    /// New content for the message (full replacement).
    pub content: Vec<ContentBlock>,
    /// New `details` — only for `function_result` / `custom` messages.
    pub details: Option<Value>,
    /// Final token/cost accounting — only for `assistant` messages. Omit for
    /// streaming content updates that do not yet have terminal usage.
    pub usage: Option<Usage>,
    /// Optimistic concurrency: when supplied and it does not match the
    /// entry's current revision, nothing is written and
    /// `{ updated: false, revision }` returns the current revision.
    pub expected_revision: Option<u64>,
    /// Opaque correlation echoed on the event.
    pub origin: Option<JsonMap>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UpdateMessageResponse {
    pub updated: bool,
    /// The entry's revision after this call (unchanged on a mismatch).
    pub revision: u64,
}

pub async fn handle(
    deps: &Deps,
    req: UpdateMessageRequest,
) -> Result<UpdateMessageResponse, SessionError> {
    let (resp, events) = deps.service.update_message(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
