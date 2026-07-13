//! `browser::evaluate` — run a JavaScript expression in the page.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EvaluateInput {
    pub session_id: String,
    /// JavaScript expression evaluated in the page. The completion value is
    /// returned by value; wrap statements in an IIFE.
    pub expression: String,
    /// Upper bound on evaluation; clamped to `max_timeout_ms`.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EvaluateOutput {
    pub ok: bool,
    /// JSON completion value when `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Exception text when not `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
