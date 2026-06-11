//! `session::delete` — delete a session and its entries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DeleteResponse {
    /// False when the session did not exist (no event fires).
    pub deleted: bool,
}

pub async fn handle(deps: &Deps, req: DeleteRequest) -> Result<DeleteResponse, SessionError> {
    let (resp, events) = deps.service.delete(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
