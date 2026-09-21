//! Per-call transport controls; defaults track the official TypeSafe SDK.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RequestOptions {
    pub headers: BTreeMap<String, String>,
    pub retry: RetryPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub attempt_timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RetryPolicy {
    #[schemars(range(max = 10))]
    pub max_retries: u32,
    pub backoff_initial_ms: u64,
    pub backoff_max_ms: u64,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub backoff_jitter: f64,
    pub http_statuses: Vec<u16>,
    pub api_connection_error: bool,
    pub api_timeout_error: bool,
    pub respect_retry_after: bool,
    pub max_retry_after_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_initial_ms: 500,
            backoff_max_ms: 5000,
            backoff_jitter: 0.25,
            http_statuses: [408, 429].into_iter().chain(500..=599).collect(),
            api_connection_error: true,
            api_timeout_error: true,
            respect_retry_after: true,
            max_retry_after_ms: 60_000,
        }
    }
}
