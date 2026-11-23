//! `session::patch-meta` — merge title/description/metadata keys on the server.
//!
//! `session::set-meta` replaces the whole metadata map, so two writers that
//! each read, change one key and write back lose one another's change. A patch
//! names only the keys it changes: each key in `set` is replaced whole, each
//! key in `unset` is removed, and every other key stays as stored. The merge
//! runs under the session's write lock, so concurrent patches of different
//! keys all land.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{JsonMap, SessionMeta};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PatchMetaRequest {
    pub session_id: String,
    /// New title (omit to keep the current one).
    pub title: Option<String>,
    /// New description (omit to keep the current one).
    pub description: Option<String>,
    /// Top-level metadata keys to write; each value replaces that key whole.
    /// Keys not named here or in `unset` are left as stored.
    pub set: Option<JsonMap>,
    /// Top-level metadata keys to remove. A key named in both `set` and
    /// `unset` is refused (`session/invalid_request`).
    pub unset: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PatchMetaResponse {
    /// The session after the merge.
    pub meta: SessionMeta,
}

pub async fn handle(deps: &Deps, req: PatchMetaRequest) -> Result<PatchMetaResponse, SessionError> {
    let (resp, events) = deps.service.patch_meta(req).await?;
    deps.sink.publish_all(&events).await;
    Ok(resp)
}
