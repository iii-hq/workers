use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PolicyMatch {
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub feature: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyAction {
    /// Opaque model identifier. Pass "auto" to defer to the classifier.
    /// Router never interprets this — the downstream gateway does.
    pub model: String,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub max_cost_per_request_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "match", alias = "match_rule")]
    pub match_rule: PolicyMatch,
    pub action: PolicyAction,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at_ms: u64,
}

fn default_priority() -> i32 {
    100
}
fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingRequest {
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub feature: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub budget_remaining_usd: Option<f64>,
    #[serde(default)]
    pub latency_slo_ms: Option<u64>,
    #[serde(default)]
    pub min_quality: Option<String>,
    #[serde(default)]
    pub classifier_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub model: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ab_test_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHealth {
    pub model: String,
    #[serde(default = "default_available")]
    pub available: bool,
    #[serde(default)]
    pub latency_p99_ms: Option<u64>,
    #[serde(default)]
    pub error_rate: Option<f64>,
    #[serde(default)]
    pub last_checked_ms: u64,
}

fn default_available() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbVariant {
    pub model: String,
    pub weight: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbTest {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "match", alias = "match_rule")]
    pub match_rule: PolicyMatch,
    pub variants: Vec<AbVariant>,
    #[serde(default = "default_metric")]
    pub metric: String,
    #[serde(default = "default_min_samples")]
    pub min_samples: u32,
    #[serde(default = "default_max_days")]
    pub max_duration_days: u32,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub created_at_ms: u64,
}

fn default_metric() -> String {
    "quality_score".to_string()
}
fn default_min_samples() -> u32 {
    100
}
fn default_max_days() -> u32 {
    14
}
fn default_status() -> String {
    "running".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbEvent {
    pub test_id: String,
    pub variant_model: String,
    pub quality_score: f64,
    pub latency_ms: u64,
    pub cost_usd: f64,
    pub recorded_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingLogEntry {
    pub timestamp_ms: u64,
    pub request_id: String,
    pub tenant: Option<String>,
    pub feature: Option<String>,
    pub model_selected: String,
    pub policy_matched: Option<String>,
    pub ab_test_id: Option<String>,
    pub reason: String,
    pub cost_usd: Option<f64>,
}

/// Classifier that maps prompt-complexity categories to user-chosen model IDs.
/// Router ships with a simple prompt heuristic — the `thresholds` map is what
/// the user controls to keep the router unopinionated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierConfig {
    pub id: String,
    /// `simple`, `moderate`, `complex`, `expert` → any opaque model ID the gateway understands.
    pub thresholds: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub created_at_ms: u64,
}

/// A model registration. The router does NOT know any model names out-of-the-box.
/// Users register whatever model IDs their gateway supports, with optional
/// quality/pricing attributes used only for the downgrade and stats paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRegistration {
    pub model: String,
    /// Any label the user wants: `low`, `medium`, `high`, `flagship`, etc.
    /// Used only when the request specifies `min_quality`.
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub input_per_1m: Option<f64>,
    #[serde(default)]
    pub output_per_1m: Option<f64>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub registered_at_ms: u64,
}
