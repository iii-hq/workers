//! `session::set-active-leaf` — switch the active path (branch switch).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetActiveLeafRequest {
    pub session_id: String,
    /// The entry the active path should end at. Subsequent
    /// `session::append` without `parent_id` chains from here.
    pub entry_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SetActiveLeafResponse {
    pub active_leaf: String,
}

pub async fn handle(
    deps: &Deps,
    req: SetActiveLeafRequest,
) -> Result<SetActiveLeafResponse, SessionError> {
    let (resp, _) = deps.service.set_active_leaf(req).await?;
    Ok(resp)
}
