//! `session::get-attachment` — read one attachment's metadata and bytes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::AttachmentMeta;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetAttachmentRequest {
    pub session_id: String,
    pub attachment_id: String,
    /// Include the bytes (`data`, standard base64). Default true; pass
    /// false for a metadata-only read.
    pub include_data: Option<bool>,
}

/// `null` when the session or attachment is unknown.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetAttachmentResponse {
    pub attachment: AttachmentMeta,
    /// Standard base64 of the bytes; `null` when `include_data` was false.
    pub data: Option<String>,
}

pub async fn handle(
    deps: &Deps,
    req: GetAttachmentRequest,
) -> Result<Option<GetAttachmentResponse>, SessionError> {
    deps.service.get_attachment(req).await
}
