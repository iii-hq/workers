//! `session::list` — list sessions with pagination/ordering.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::types::{JsonMap, SessionMeta, SessionStatus};

/// Sort order for `session::list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListOrder {
    CreatedAsc,
    CreatedDesc,
    #[default]
    UpdatedDesc,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Page size. Default 50, clamped to the configured maximum.
    pub limit: Option<usize>,
    /// Opaque pagination cursor from a previous response.
    pub cursor: Option<String>,
    /// Only sessions with this status.
    pub status: Option<SessionStatus>,
    /// Equality filter against `SessionMeta.metadata` (tenancy):
    /// every given key must equal the stored value.
    pub metadata: Option<JsonMap>,
    /// Sort order. Default `updated_desc`.
    pub order: Option<ListOrder>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListResponse {
    pub sessions: Vec<SessionMeta>,
    /// Present when more pages remain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub async fn handle(deps: &Deps, req: ListRequest) -> Result<ListResponse, SessionError> {
    let (resp, _) = deps.service.list(req).await?;
    Ok(resp)
}
