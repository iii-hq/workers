//! Trusted workspace grant controls. These are registered harness functions for
//! orchestration code, but intentionally excluded from the model-facing catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct WorkspaceGrantRequest {
    pub session_id: String,
    pub root: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct WorkspaceGrantsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct WorkspaceRevokeRequest {
    pub session_id: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceGrantsResponse {
    pub session_id: String,
    pub roots: Vec<String>,
}

pub async fn grant(
    deps: &Deps,
    req: WorkspaceGrantRequest,
) -> Result<WorkspaceGrantsResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let roots = crate::workspace_grants::grant(
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
    req: WorkspaceGrantsRequest,
) -> Result<WorkspaceGrantsResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let roots =
        crate::workspace_grants::roots(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    Ok(response(req.session_id, roots))
}

pub async fn revoke(
    deps: &Deps,
    req: WorkspaceRevokeRequest,
) -> Result<WorkspaceGrantsResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let roots = crate::workspace_grants::revoke(
        &deps.iii,
        &req.session_id,
        &req.root,
        cfg.session_timeout_ms,
    )
    .await?;
    Ok(response(req.session_id, roots))
}

fn response(session_id: String, roots: Vec<String>) -> WorkspaceGrantsResponse {
    WorkspaceGrantsResponse { session_id, roots }
}
