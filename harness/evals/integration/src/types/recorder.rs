//! Recorder configuration and durable event types. Configuration and
//! evidence collection are in-process runner operations; only controlled
//! target functions and the lifecycle sink are exposed through the engine.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::script::SchemaVersion1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderConfigV1 {
    pub target: RecorderTargetV1,
    pub lifecycle: RecorderLifecycleV1,
    /// Additional run-scoped controlled functions (e.g. hook implementations
    /// for `harness::hook::*` scenarios). Registered exactly like the target:
    /// declared description/schema verbatim, every call durably recorded,
    /// declared response returned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_functions: Vec<RecorderTargetV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderTargetV1 {
    /// Must be prefixed by `<run_id>::` — enforced by `configure`.
    pub function_id: String,
    pub description: String,
    /// Registered verbatim; the same schema must appear in native tool
    /// exposure (that registration is part of the oracle).
    pub request_schema: serde_json::Map<String, serde_json::Value>,
    /// Declared response returned for every target call.
    pub response: serde_json::Value,
    /// One-based call ordinal whose response is held behind a runner-owned,
    /// in-process gate after the call has been durably appended. Derived from
    /// `fault.after_target_calls`; never authored directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub hold_response_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderLifecycleV1 {
    pub trigger_type: LifecycleTriggerType,
    pub function_id: LifecycleFunctionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LifecycleTriggerType {
    #[serde(rename = "harness::turn-completed")]
    TurnCompleted,
}

impl LifecycleTriggerType {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleTriggerType::TurnCompleted => "harness::turn-completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LifecycleFunctionId {
    #[serde(rename = "integration-recorder::lifecycle")]
    Lifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecorderEventKind {
    TargetCall,
    Lifecycle,
}

/// Exact wire shape accepted by the private lifecycle sink. The harness
/// trigger can attach optional parent/reaction/result context, but the
/// session, turn, terminal status, and timestamp are always present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEventV1 {
    pub session_id: String,
    pub turn_id: String,
    pub status: LifecycleStatusV1,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderEventV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    /// Strictly increasing; assigned before the handler responds.
    pub sequence: u64,
    pub kind: RecorderEventKind,
    pub function_id: String,
    pub payload: serde_json::Value,
    /// RFC 3339 receipt time — diagnostic, never an oracle input.
    pub received_at: String,
}
