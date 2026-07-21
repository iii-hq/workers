use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::frames::Usage;
use crate::types::script::{JsonMatcherV1, ModelFixtureV1};

use super::ExpectationsV1;

/// The authored scenario data built by the `src/scenarios` builder modules.
///
/// This layer is code, never serialized: there is no schema pair to keep
/// synchronized and no round trip. `DeadlinesV1`, `ReleaseV1`, and
/// `FaultKind` below are shared with the compiled layer and keep their wire
/// derives.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredScenarioV1 {
    pub id: String,
    pub description: String,
    pub quarantine: bool,
    pub send: ScenarioSendV1,
    /// Alias → controlled function. Aliases are expanded to
    /// `{{run_id}}::<alias>` by the compiler.
    pub functions: BTreeMap<String, ScenarioFunctionV1>,
    pub router: ScenarioRouterV1,
    pub bindings: Vec<TriggerBindingSpecV1>,
    pub release: Option<ReleaseV1>,
    pub fault: Option<FaultV1>,
    pub timeouts: DeadlinesV1,
    pub expect: ExpectationsV1,
}

/// Scenario ids are also artifact directory names, so keep them to one safe,
/// portable path component.
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

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioSendV1 {
    pub message: String,
    /// Allowed function aliases. Omitted means every function whose
    /// `expose` flag is true; an empty list disables function dispatch.
    pub allow: Option<Vec<String>>,
    /// Omitted values are derived deterministically from the scenario id.
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioFunctionV1 {
    pub description: String,
    pub request_schema: serde_json::Map<String, serde_json::Value>,
    pub response: serde_json::Value,
    /// Exposed to the model by default. Hook-only controlled functions set
    /// this to false.
    pub expose: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioRouterV1 {
    /// Omitted for the deterministic `fixture-model` / `scripted` catalog
    /// entry used by the integration stack.
    pub model: Option<ModelFixtureV1>,
    pub generations: Vec<ScenarioGenerationV1>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioGenerationV1 {
    pub reply: RouterReplyV1,
    /// Escape hatch for fields whose history is intentionally unstable, such
    /// as the post-crash request in a recovery reproduction.
    pub match_overrides: GenerationMatchOverridesV1,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouterReplyV1 {
    Text {
        text: String,
        /// Non-empty chunks produce the complete streaming frame sequence.
        /// Omitted chunks produce one terminal `done` frame.
        chunks: Vec<String>,
        usage: Option<Usage>,
    },
    FunctionCall {
        /// Defaults to `call-<function-call ordinal>`.
        id: Option<String>,
        /// Function alias from `functions`.
        function: String,
        arguments: serde_json::Value,
        usage: Option<Usage>,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenerationMatchOverridesV1 {
    pub writer_ref: Option<JsonMatcherV1>,
    pub request_id: Option<JsonMatcherV1>,
    pub model: Option<JsonMatcherV1>,
    pub provider: Option<JsonMatcherV1>,
    pub system_prompt: Option<JsonMatcherV1>,
    pub messages: Option<JsonMatcherV1>,
    pub tools: Option<JsonMatcherV1>,
    pub response_format: Option<JsonMatcherV1>,
    pub thinking_level: Option<JsonMatcherV1>,
    pub max_output_tokens: Option<JsonMatcherV1>,
    pub provider_options: Option<JsonMatcherV1>,
    pub metadata: Option<JsonMatcherV1>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerBindingSpecV1 {
    pub trigger: TriggerKindV1,
    /// Controlled function alias invoked by the trigger.
    pub function: String,
    /// Exposed function aliases selected by this hook.
    pub functions: Vec<String>,
    pub priority: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKindV1 {
    HookPreTrigger,
}

impl TriggerKindV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HookPreTrigger => "harness::hook::pre-trigger",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultV1 {
    pub kind: FaultKind,
    /// Controlled function alias to interrupt. Omitted means the first
    /// authored function call.
    pub function: Option<String>,
    pub after_target_calls: u64,
    pub restart_delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    EngineSigkill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseV1 {
    #[serde(
        default = "default_call_id",
        skip_serializing_if = "is_default_call_id"
    )]
    pub function_call_id: String,
    pub action: ReleaseActionV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseActionV1 {
    Execute,
    Deliver,
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
            scenario_ms: 60_000,
            teardown_ms: 15_000,
        }
    }
}

pub(super) fn default_call_id() -> String {
    "call-1".to_string()
}

fn is_default_call_id(value: &str) -> bool {
    value == default_call_id()
}
