use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// "minimal" requests the lowest reasoning effort and needs only `thinking`
/// support; levels map to provider-native knobs via `Model::thinking_budgets`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pricing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

/// The capability record (README § Model descriptor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Model {
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
    pub supports_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_structured_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<BTreeMap<ThinkingLevel, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
}

/// Function invocation schema — what a provider sees as a `tools` array entry
/// (README § Function invocation schema; adapter boundary). These describe iii
/// functions exposed to the model, not provider-native tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema of the arguments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>, // "parallel" | "sequential"
}
