use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::recorder::RecorderConfigV1;
use crate::types::script::SchemaVersion1;

use super::{DeadlinesV1, FaultKind, InvariantSpecV1, ReleaseV1};

/// Strict runtime scenario produced from [`super::IntegrationScenarioV1`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledScenarioV1 {
    pub schema_version: SchemaVersion1,
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]+$"))]
    pub id: String,
    pub description: String,
    pub send: serde_json::Value,
    pub recorder: RecorderConfigV1,
    pub deadlines: DeadlinesV1,
    pub invariants: Vec<InvariantSpecV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<CompiledFaultV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<TriggerBindingV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseV1>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quarantine: bool,
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
