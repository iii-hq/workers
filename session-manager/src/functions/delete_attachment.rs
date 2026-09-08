//! `session::delete-attachment` — remove one stored attachment that no
//! transcript entry references (typically a chip removed from the composer
//! before sending). Event-silent; also drops the attachment from the parked
//! draft.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeleteAttachmentRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DeleteAttachmentResponse {
    /// `false` when the attachment was already absent (a silent no-op).
    pub deleted: bool,
}

pub async fn handle(
    deps: &Deps,
    req: DeleteAttachmentRequest,
) -> Result<DeleteAttachmentResponse, SessionError> {
    deps.service.delete_attachment(req).await
}
