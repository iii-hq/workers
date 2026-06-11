//! `session::append` — append one entry, idempotent on `entry_id`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{AgentMessage, CustomPayload, JsonMap};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AppendRequest {
    pub session_id: String,
    /// The message to append (a `kind: "message"` entry). Exactly one
    /// of `message` / `custom` must be supplied.
    pub message: Option<AgentMessage>,
    /// Append a bookkeeping `kind: "custom"` entry instead of a message
    /// (e.g. the harness's compaction record). Custom entries do not
    /// count toward `message_count` and are only returned by
    /// `session::messages` when `include_custom` is set.
    pub custom: Option<CustomPayload>,
    /// Override the parent (default: active leaf). Appending always
    /// moves the active leaf to the new entry.
    pub parent_id: Option<String>,
    /// Caller-supplied id for idempotent appends: appending an id that
    /// already exists is a no-op — the existing entry is returned and
    /// no event fires.
    pub entry_id: Option<String>,
    /// Opaque correlation (e.g. `{ "turn_id": ... }`), echoed on events.
    pub origin: Option<JsonMap>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AppendResponse {
    pub entry_id: String,
    pub parent_id: Option<String>,
    pub timestamp: i64,
}

pub async fn handle(deps: &Deps, req: AppendRequest) -> Result<AppendResponse, SessionError> {
    let (resp, events) = deps.service.append(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
