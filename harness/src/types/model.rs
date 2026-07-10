//! Model descriptor and function-invocation schema (README § Model
//! descriptor / § Function invocation schema). Wire-aligned with
//! `llm-router`'s catalog so `router::models::get` parses back into `Model`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// One provider-native reasoning effort advertised for a specific model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReasoningEffort {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    pub reasoning_efforts: Option<Vec<ReasoningEffort>>,
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

/// Function invocation schema — one provider `tools` array entry. The
/// harness attaches the single `agent_trigger` entry by default, or one
/// per allowed function in native exposure mode, plus the synthetic
/// `submit_result` entry for an output contract using the fallback strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
}
