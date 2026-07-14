//! `session::set-draft` — park (or clear) the session's unsent composer
//! input so a client reload restores what the user was typing.
//!
//! Deliberately event-silent and `updated_at`-neutral: drafts are written at
//! keystroke cadence, and a save must neither re-order `session::list` nor
//! spam `session::meta-updated` subscribers. The draft reads back on the
//! `SessionMeta` returned by `session::get` / `session::list`; it is never
//! touched by `session::set-meta`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetDraftRequest {
    pub session_id: String,
    /// The unsent input to park. Omit, or send empty/whitespace-only text,
    /// to clear the stored draft.
    pub draft: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SetDraftResponse {
    /// The draft as stored after this call (`null` when cleared).
    pub draft: Option<String>,
}

pub async fn handle(deps: &Deps, req: SetDraftRequest) -> Result<SetDraftResponse, SessionError> {
    let (resp, _) = deps.service.set_draft(req).await?;
    Ok(resp)
}
