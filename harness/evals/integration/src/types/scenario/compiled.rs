use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::recorder::RecorderConfigV1;
use crate::types::script::SchemaVersion1;

use super::{DeadlinesV1, FaultKind, ReleaseV1};

/// Strict runtime scenario produced from [`super::AuthoredScenarioV1`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledScenarioV1 {
    pub schema_version: SchemaVersion1,
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]+$"))]
    pub id: String,
    pub description: String,
    pub send: CompiledSendV1,
    pub recorder: RecorderConfigV1,
    pub deadlines: DeadlinesV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<CompiledFaultV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<TriggerBindingV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseV1>,
}

/// The deliberately narrow `harness::send` request emitted by the scenario
/// compiler. Keeping this typed prevents compiler changes from silently
/// introducing fields outside the live harness contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledSendV1 {
    pub session_id: String,
    pub message: String,
    pub model: String,
    pub provider: String,
    pub idempotency_key: String,
    pub options: CompiledSendOptionsV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledSendOptionsV1 {
    pub functions: CompiledFunctionPolicyV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledFunctionPolicyV1 {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub expose: CompiledFunctionExposureV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompiledFunctionExposureV1 {
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledFaultV1 {
    pub kind: FaultKind,
    pub function_id: String,
    #[schemars(range(min = 1))]
    pub after_target_calls: u64,
    pub restart_delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriggerBindingV1 {
    pub trigger_type: String,
    pub function_id: String,
    pub config: serde_json::Value,
}
