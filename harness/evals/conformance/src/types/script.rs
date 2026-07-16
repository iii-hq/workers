//! `RouterScriptV1` — the strict scripted-router fixture (spec § Proposed
//! scripted-router contract). Loaded, expanded, and validated before the
//! stack boots; any defect here is a `runner_error`, never a subject failure.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::frames::{AssistantMessageEvent, RouterChatResponse};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouterScriptV1 {
    pub schema_version: SchemaVersion1,
    pub scenario_id: String,
    pub model: ModelFixtureV1,
    pub generations: Vec<ScriptedGenerationV1>,
}

/// The literal string `"1"`; any other value is a schema error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SchemaVersion1 {
    #[serde(rename = "1")]
    V1,
}

/// Mirror of the catalog `Model` (`llm-router/src/types/model.rs:43`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelFixtureV1 {
    pub id: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub context_window: u64,
    pub max_output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_xhigh: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<ReasoningEffortV1>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_structured_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<std::collections::BTreeMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<PricingV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReasoningEffortV1 {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PricingV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonMatcherV1 {
    Absent,
    Present,
    Regex {
        pattern: String,
    },
    Sha256 {
        expected: String,
    },
    Exact {
        expected: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        normalize: Option<Vec<JsonNormalizerV1>>,
    },
    Subset {
        expected: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        normalize: Option<Vec<JsonNormalizerV1>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonNormalizerV1 {
    /// RFC 6901 JSON Pointer.
    pub pointer: String,
    pub operation: NormalizerOperation,
    /// Required for `replace`; forbidden for `delete`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NormalizerOperation {
    Delete,
    Replace,
}

/// The 12 router-request fields; every one carries an explicit matcher —
/// there is no runner default (spec § First implementation slice).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerationMatchV1 {
    pub writer_ref: JsonMatcherV1,
    pub request_id: JsonMatcherV1,
    pub model: JsonMatcherV1,
    pub provider: JsonMatcherV1,
    pub system_prompt: JsonMatcherV1,
    pub messages: JsonMatcherV1,
    pub tools: JsonMatcherV1,
    pub response_format: JsonMatcherV1,
    pub thinking_level: JsonMatcherV1,
    pub max_output_tokens: JsonMatcherV1,
    pub provider_options: JsonMatcherV1,
    pub metadata: JsonMatcherV1,
}

pub const MATCH_FIELDS: [&str; 12] = [
    "writer_ref",
    "request_id",
    "model",
    "provider",
    "system_prompt",
    "messages",
    "tools",
    "response_format",
    "thinking_level",
    "max_output_tokens",
    "provider_options",
    "metadata",
];

impl GenerationMatchV1 {
    /// Field name → matcher, in the canonical field order.
    pub fn fields(&self) -> [(&'static str, &JsonMatcherV1); 12] {
        [
            ("writer_ref", &self.writer_ref),
            ("request_id", &self.request_id),
            ("model", &self.model),
            ("provider", &self.provider),
            ("system_prompt", &self.system_prompt),
            ("messages", &self.messages),
            ("tools", &self.tools),
            ("response_format", &self.response_format),
            ("thinking_level", &self.thinking_level),
            ("max_output_tokens", &self.max_output_tokens),
            ("provider_options", &self.provider_options),
            ("metadata", &self.metadata),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BarrierV1 {
    /// Frame index the stream pauses before (0-based into `frames`).
    pub before_frame: usize,
    pub id: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptedGenerationV1 {
    pub ordinal: u64,
    #[serde(rename = "match")]
    pub match_: GenerationMatchV1,
    pub frames: Vec<AssistantMessageEvent>,
    pub response: RouterChatResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub barriers: Option<Vec<BarrierV1>>,
}
