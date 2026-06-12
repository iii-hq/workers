//! `session::set_meta` — update title/description/metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{JsonMap, SessionMeta};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetMetaRequest {
    pub session_id: String,
    /// New title (omit to keep the current one).
    pub title: Option<String>,
    /// New description (omit to keep the current one).
    pub description: Option<String>,
    /// New metadata object — **replaces** the stored one wholesale.
    pub metadata: Option<JsonMap>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SetMetaResponse {
    pub meta: SessionMeta,
}

pub async fn handle(deps: &Deps, req: SetMetaRequest) -> Result<SetMetaResponse, SessionError> {
    let (resp, events) = deps.service.set_meta(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
