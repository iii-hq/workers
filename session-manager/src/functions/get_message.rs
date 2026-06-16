//! `session::get-message` — read a single entry by id.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::SessionEntry;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetMessageRequest {
    pub session_id: String,
    pub entry_id: String,
}

/// `null` when the session or entry is unknown.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetMessageResponse {
    pub entry: SessionEntry,
}

pub async fn handle(
    deps: &Deps,
    req: GetMessageRequest,
) -> Result<Option<GetMessageResponse>, SessionError> {
    deps.service.get_message(req).await
}
