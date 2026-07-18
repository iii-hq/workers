//! Recorder control-plane types (spec § Proposed recorder contract). The
//! five `integration-recorder::*` functions speak exactly these shapes.

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
    /// Delay between the durable append and the response, opening a window
    /// for fault injection while the dispatched call is executing
    /// (crash-recovery scenarios). Omitted = respond immediately.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderLifecycleV1 {
    pub trigger_type: LifecycleTriggerType,
    pub function_id: LifecycleFunctionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LifecycleTriggerType {
    #[serde(rename = "harness::turn-started")]
    TurnStarted,
    #[serde(rename = "harness::turn-completed")]
    TurnCompleted,
}

impl LifecycleTriggerType {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleTriggerType::TurnStarted => "harness::turn-started",
            LifecycleTriggerType::TurnCompleted => "harness::turn-completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LifecycleFunctionId {
    #[serde(rename = "integration-recorder::lifecycle")]
    Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderConfigureRequestV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub config: RecorderConfigV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderConfigureResponseV1 {
    pub schema_version: SchemaVersion1,
    /// SHA-256 of the canonical JSON of the registered request schema.
    pub target_schema_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderResetRequestV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderResetResponseV1 {
    pub schema_version: SchemaVersion1,
    pub next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderSnapshotRequestV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderSnapshotResponseV1 {
    pub schema_version: SchemaVersion1,
    pub events: Vec<RecorderEventV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderAwaitRequestV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub kind: RecorderEventKind,
    pub count: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecorderAwaitResponseV1 {
    pub schema_version: SchemaVersion1,
    pub observed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecorderEventKind {
    TargetCall,
    Lifecycle,
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
