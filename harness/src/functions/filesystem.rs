//! Trusted filesystem grant controls. These are registered harness functions for
//! orchestration code, but intentionally excluded from the model-facing catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FilesystemGrantRequest {
    pub session_id: String,
    pub root: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FilesystemGrantsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FilesystemRevokeRequest {
    pub session_id: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FilesystemGrantsResponse {
    pub session_id: String,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FilesystemInfoRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FilesystemInfoResponse {
    /// Working-directory root stamped onto the first turn of a session whose
    /// send carries no explicit `fs_scope.root`; `null` when defaulting is
    /// disabled (`default_filesystem_root: "off"`) or the cwd is unreadable.
    pub default_root: Option<String>,
}

/// The resolved default working directory — the value a console pre-fills the
/// picker with so the folder a new chat will be scoped to is visible up front.
pub async fn info(
    deps: &Deps,
    _req: FilesystemInfoRequest,
) -> Result<FilesystemInfoResponse, HarnessError> {
    let cfg = deps.cfg().await;
    Ok(FilesystemInfoResponse {
        default_root: cfg.resolved_default_filesystem_root(),
    })
}

pub async fn grant(
    deps: &Deps,
    req: FilesystemGrantRequest,
) -> Result<FilesystemGrantsResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let roots = crate::filesystem_grants::grant(
        &deps.iii,
        &req.session_id,
        req.root,
        cfg.session_timeout_ms,
    )
    .await?;
    Ok(response(req.session_id, roots))
}

pub async fn grants(
    deps: &Deps,
    req: FilesystemGrantsRequest,
) -> Result<FilesystemGrantsResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let roots =
        crate::filesystem_grants::roots(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    Ok(response(req.session_id, roots))
}

pub async fn revoke(
    deps: &Deps,
    req: FilesystemRevokeRequest,
) -> Result<FilesystemGrantsResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let roots = crate::filesystem_grants::revoke(
        &deps.iii,
        &req.session_id,
        &req.root,
        cfg.session_timeout_ms,
    )
    .await?;
    Ok(response(req.session_id, roots))
}

fn response(session_id: String, roots: Vec<String>) -> FilesystemGrantsResponse {
    FilesystemGrantsResponse { session_id, roots }
}
