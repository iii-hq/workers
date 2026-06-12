//! `session::ensure` — idempotently ensure a session with a given id exists.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{JsonMap, SessionMeta};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnsureRequest {
    /// Caller-chosen session id to ensure.
    pub session_id: String,
    /// Title applied only when the session is created.
    pub title: Option<String>,
    /// Description applied only when the session is created.
    pub description: Option<String>,
    /// Metadata applied only when the session is created.
    pub metadata: Option<JsonMap>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct EnsureResponse {
    pub session_id: String,
    pub meta: SessionMeta,
    /// True when this call created the session (and fired `session::created`).
    pub created: bool,
}

pub async fn handle(deps: &Deps, req: EnsureRequest) -> Result<EnsureResponse, SessionError> {
    let (resp, events) = deps.service.ensure(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
