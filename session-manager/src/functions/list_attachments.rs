//! `session::list-attachments` — every attachment's metadata of a session.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::AttachmentMeta;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListAttachmentsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListAttachmentsResponse {
    /// Oldest first (`created_at`, then `attachment_id`).
    pub attachments: Vec<AttachmentMeta>,
}

pub async fn handle(
    deps: &Deps,
    req: ListAttachmentsRequest,
) -> Result<ListAttachmentsResponse, SessionError> {
    deps.service.list_attachments(req).await
}
