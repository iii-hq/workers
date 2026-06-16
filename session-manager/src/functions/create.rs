//! `session::create` — create a session at status `idle`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{JsonMap, SessionMeta};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CreateRequest {
    /// Session title; may be refined later with `session::set-meta`. Default "".
    pub title: Option<String>,
    /// Session description. Default "".
    pub description: Option<String>,
    /// App-defined metadata persisted onto `SessionMeta` — the tenancy
    /// hook (e.g. `{ "owner": "u_1" }`) that `session::list` and every
    /// trigger config can filter on.
    pub metadata: Option<JsonMap>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreateResponse {
    pub session_id: String,
    pub meta: SessionMeta,
}

pub async fn handle(deps: &Deps, req: CreateRequest) -> Result<CreateResponse, SessionError> {
    let (resp, events) = deps.service.create(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
