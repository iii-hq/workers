//! `session::get` — read one session's metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::SessionMeta;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetRequest {
    pub session_id: String,
}

/// `null` when the session is unknown.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetResponse {
    pub meta: SessionMeta,
}

pub async fn handle(deps: &Deps, req: GetRequest) -> Result<Option<GetResponse>, SessionError> {
    deps.service.get(req).await
}
