use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::probe::ControlledTargetV1;
use crate::types::script::SchemaVersion1;

pub fn validate_scenario_id(id: &str) -> anyhow::Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    anyhow::ensure!(
        valid,
        "scenario id must be 1-128 ASCII letters, digits, '-' or '_'"
    );
    Ok(())
}

/// Strict runtime scenario consumed by the integration runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledScenarioV1 {
    pub schema_version: SchemaVersion1,
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]+$"))]
    pub id: String,
    pub description: String,
    pub send: CompiledSendV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ControlledTargetV1>,
    pub deadlines: DeadlinesV1,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CompiledSendOptionsV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledSendOptionsV1 {
    pub functions: CompiledFunctionPolicyV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledFunctionPolicyV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    pub deny: Vec<String>,
    pub expose: CompiledFunctionExposureV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompiledFunctionExposureV1 {
    AgentTrigger,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeadlinesV1 {
    #[schemars(range(min = 1))]
    pub readiness_ms: u64,
    #[schemars(range(min = 1))]
    pub scenario_ms: u64,
    #[schemars(range(min = 1))]
    pub teardown_ms: u64,
}

impl Default for DeadlinesV1 {
    fn default() -> Self {
        Self {
            readiness_ms: 60_000,
            // Stack boot is budgeted by `readiness_ms`; this covers only the
            // await phase, where the slowest passing scenario observed in CI
            // takes ~4s. 25s keeps ~6x headroom while capping what a wedged
            // scenario costs — at 60s, a red run spent more time waiting on
            // deadlines than doing work.
            scenario_ms: 25_000,
            teardown_ms: 15_000,
        }
    }
}
