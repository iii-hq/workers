//! Controlled function and lifecycle contracts used by the scenario probe.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlledTargetV1 {
    /// Must be prefixed by `<run_id>::` — enforced before registration.
    pub function_id: String,
    pub description: String,
    pub request_schema: serde_json::Map<String, serde_json::Value>,
    pub response: serde_json::Value,
}

/// Exact wire shape accepted by the private lifecycle sink.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEventV1 {
    pub session_id: String,
    pub turn_id: String,
    pub status: LifecycleStatusV1,
    pub terminal: bool,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<LifecycleParentV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reactive_depth: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatusV1 {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleParentV1 {
    pub session_id: String,
    pub turn_id: String,
    pub function_call_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleAcceptedV1 {
    pub accepted: bool,
}
