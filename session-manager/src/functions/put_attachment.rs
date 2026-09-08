//! `session::put-attachment` — store one attachment's original bytes and
//! hand back the reference block a message embeds to point at it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{AttachmentMeta, ContentBlock};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutAttachmentRequest {
    pub session_id: String,
    /// Original filename (non-empty).
    pub name: String,
    /// MIME type; pass `application/octet-stream` when unknown.
    pub mime: String,
    /// The bytes as standard (padded) base64. Decoded size must not exceed
    /// the configured `max_attachment_bytes`.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PutAttachmentResponse {
    pub attachment: AttachmentMeta,
    /// Ready-made `type: "file"` content block referencing the attachment,
    /// to embed in the message's `content`.
    pub block: ContentBlock,
}

pub async fn handle(
    deps: &Deps,
    req: PutAttachmentRequest,
) -> Result<PutAttachmentResponse, SessionError> {
    // Event-silent: the attachment only becomes visible through the
    // message that references it, and that append fires its own event.
    deps.service.put_attachment(req).await
}
