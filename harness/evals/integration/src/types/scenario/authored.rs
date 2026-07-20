use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::frames::{AssistantMessageEvent, RouterChatResponse, Usage};
use crate::types::script::{JsonMatcherV1, ModelFixtureV1, SchemaVersion1};

use super::ExpectationsV1;

/// The single-file scenario authors maintain in `scenarios/<slug>/scenario.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthoredScenarioV1 {
    pub schema_version: SchemaVersion1,
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]+$"))]
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quarantine: bool,
    pub send: ScenarioSendV1,
    /// Alias → controlled function. Aliases are expanded to
    /// `{{run_id}}::<alias>` by the compiler.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(schema_with = "functions_schema")]
    pub functions: BTreeMap<String, ScenarioFunctionV1>,
    pub router: ScenarioRouterV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<TriggerBindingSpecV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<FaultV1>,
    #[serde(default, skip_serializing_if = "DeadlinesV1::is_default")]
    pub timeouts: DeadlinesV1,
    #[serde(default, skip_serializing_if = "ExpectationsV1::is_default")]
    pub expect: ExpectationsV1,
}

/// Compatibility name retained for callers compiled against the first
/// single-file authoring API. New code should use [`AuthoredScenarioV1`].
pub type IntegrationScenarioV1 = AuthoredScenarioV1;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSendV1 {
    pub message: String,
    /// Allowed function aliases. Omitted means every function whose
    /// `expose` flag is true; an empty list disables function dispatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    /// Omitted values are derived deterministically from the scenario id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFunctionV1 {
    pub description: String,
    pub request_schema: serde_json::Map<String, serde_json::Value>,
    pub response: serde_json::Value,
    /// Exposed to the model by default. Hook-only controlled functions set
    /// this to false.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub expose: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioRouterV1 {
    /// Omitted for the deterministic `fixture-model` / `scripted` catalog
    /// entry used by the integration stack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelFixtureV1>,
    pub generations: Vec<ScenarioGenerationV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioGenerationV1 {
    pub reply: RouterReplyV1,
    /// Escape hatch for fields whose history is intentionally unstable, such
    /// as the post-crash request in a recovery reproduction.
    #[serde(default, skip_serializing_if = "GenerationMatchOverridesV1::is_empty")]
    pub match_overrides: GenerationMatchOverridesV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouterReplyV1 {
    Text {
        text: String,
        /// Non-empty chunks produce the complete streaming frame sequence.
        /// Omitted chunks produce one terminal `done` frame.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        chunks: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    FunctionCall {
        /// Defaults to `call-<function-call ordinal>`.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Function alias from `functions`.
        function: String,
        arguments: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// Full strict wire contract for cases the typed replies cannot express.
    Raw {
        frames: Vec<AssistantMessageEvent>,
        response: RouterChatResponse,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerationMatchOverridesV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer_ref: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<JsonMatcherV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMatcherV1>,
}

impl GenerationMatchOverridesV1 {
    pub fn is_empty(&self) -> bool {
        self.writer_ref.is_none()
            && self.request_id.is_none()
            && self.model.is_none()
            && self.provider.is_none()
            && self.system_prompt.is_none()
            && self.messages.is_none()
            && self.tools.is_none()
            && self.response_format.is_none()
            && self.thinking_level.is_none()
            && self.max_output_tokens.is_none()
            && self.provider_options.is_none()
            && self.metadata.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriggerBindingSpecV1 {
    pub trigger: TriggerKindV1,
    /// Controlled function alias invoked by the trigger.
    pub function: String,
    /// Exposed function aliases selected by this hook.
    pub functions: Vec<String>,
    pub priority: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultV1 {
    pub kind: FaultKind,
    /// Controlled function alias to interrupt. Omitted means the first
    /// authored function call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(default = "default_one", skip_serializing_if = "is_one")]
    #[schemars(range(min = 1))]
    pub after_target_calls: u64,
    #[serde(
        default = "default_restart_delay_ms",
        skip_serializing_if = "is_default_restart_delay_ms"
    )]
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
    #[serde(default = "default_readiness_ms")]
    #[schemars(range(min = 1))]
    pub readiness_ms: u64,
    #[serde(default = "default_scenario_ms")]
    #[schemars(range(min = 1))]
    pub scenario_ms: u64,
    #[serde(default = "default_teardown_ms")]
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

impl DeadlinesV1 {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct FunctionAliasSchema(
    #[schemars(length(min = 1), regex(pattern = "^[A-Za-z0-9_-]+$"))] String,
);

fn functions_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
    let mut schema: schemars::schema::SchemaObject =
        BTreeMap::<String, ScenarioFunctionV1>::json_schema(generator).into();
    schema
        .object
        .as_mut()
        .expect("map schema has object validation")
        .property_names = Some(Box::new(FunctionAliasSchema::json_schema(generator)));
    schema.into()
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_one() -> u64 {
    1
}

fn is_one(value: &u64) -> bool {
    *value == default_one()
}

fn default_restart_delay_ms() -> u64 {
    1_500
}

fn is_default_restart_delay_ms(value: &u64) -> bool {
    *value == default_restart_delay_ms()
}

fn default_readiness_ms() -> u64 {
    60_000
}

fn default_scenario_ms() -> u64 {
    60_000
}

fn default_teardown_ms() -> u64 {
    15_000
}

pub(super) fn default_call_id() -> String {
    "call-1".to_string()
}

fn is_default_call_id(value: &str) -> bool {
    value == default_call_id()
}
