//! `session::set-draft` — park (or clear) the session's unsent composer
//! input — text and attachments — so a client reload restores what the user
//! was composing.
//!
//! Deliberately event-silent and `updated_at`-neutral: drafts are written at
//! keystroke cadence, and a save must neither re-order `session::list` nor
//! spam `session::meta-updated` subscribers. The draft reads back on the
//! `SessionMeta` returned by `session::get` / `session::list` (`draft`,
//! `draft_attachments`); it is never touched by `session::set-meta`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::AttachmentMeta;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetDraftRequest {
    pub session_id: String,
    /// The unsent input to park. Omit, or send empty/whitespace-only text,
    /// to clear the stored draft text.
    pub draft: Option<String>,
    /// Attachments parked with the draft, by id (uploaded beforehand with
    /// `session::put-attachment`; an unknown id is `session/invalid_request`).
    /// Omit to leave the parked attachments unchanged — a text-only save must
    /// not drop them — or send `[]` to clear them.
    pub attachment_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SetDraftResponse {
    /// The draft text as stored after this call (`null` when cleared).
    pub draft: Option<String>,
    /// The parked attachments as stored after this call, in request order.
    pub attachments: Vec<AttachmentMeta>,
}

pub async fn handle(deps: &Deps, req: SetDraftRequest) -> Result<SetDraftResponse, SessionError> {
    let (resp, _) = deps.service.set_draft(req).await?;
    Ok(resp)
}
