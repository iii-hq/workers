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
