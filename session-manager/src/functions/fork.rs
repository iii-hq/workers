//! `session::fork` — copy history up to an entry into a new session.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::SessionMeta;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ForkRequest {
    /// Source session.
    pub session_id: String,
    /// Fork point: the root -> entry path is copied (copy-on-fork,
    /// fresh entry ids) and becomes the new session's active path.
    pub entry_id: String,
    /// Title for the new session (default: the source's title).
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ForkResponse {
    /// The new session's id (`forked_from` is set on its meta).
    pub session_id: String,
    pub meta: SessionMeta,
}

pub async fn handle(deps: &Deps, req: ForkRequest) -> Result<ForkResponse, SessionError> {
    let (resp, events) = deps.service.fork(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
