//! `harness::function::resolve` — settle a `pending` call and resume the
//! parked turn (harness.md § `harness::function::resolve`). Called by the
//! harness itself (child completion) or a trusted sibling (an approval
//! decision); denied to in-run agents.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::content::ContentBlock;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ResolveFsScope {
    #[serde(default)]
    pub grants: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FunctionResolveRequest {
    pub session_id: String,
    pub turn_id: String,
    pub function_call_id: String,
    /// `deliver` (default) supplies the result; `execute` releases a
    /// hook-held call through the remaining trigger pipeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// execute only: one-shot additional roots trusted by the caller and
    /// unioned with the session's durable filesystem grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs_scope: Option<ResolveFsScope>,
    // deliver only:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentBlock>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FunctionResolveResponse {
    /// False when the call is unknown, already done, or (execute) not held.
    pub resolved: bool,
    /// True when this resolve re-enqueued the turn.
    pub turn_resumed: bool,
}

pub async fn handle(
    deps: &Deps,
    req: FunctionResolveRequest,
) -> Result<FunctionResolveResponse, HarnessError> {
    crate::deferred::resolve(deps, req).await
}
