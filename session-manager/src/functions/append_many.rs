//! `session::append_many` — append several message entries in order.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{AgentMessage, JsonMap};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AppendManyRequest {
    pub session_id: String,
    /// Messages appended in order, each chained to the previous one.
    /// Not idempotent — use `session::append` with `entry_id` where
    /// redelivery is possible.
    pub messages: Vec<AgentMessage>,
    /// Parent of the first appended entry (default: active leaf).
    pub parent_id: Option<String>,
    /// Opaque correlation echoed on every emitted event.
    pub origin: Option<JsonMap>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AppendManyResponse {
    pub entry_ids: Vec<String>,
    pub last_entry_id: String,
}

pub async fn handle(
    deps: &Deps,
    req: AppendManyRequest,
) -> Result<AppendManyResponse, SessionError> {
    let (resp, events) = deps.service.append_many(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
