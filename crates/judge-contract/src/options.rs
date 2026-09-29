//! Per-call transport controls. Retry policy is the provider's, not the caller's.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RequestOptions {
    /// Bounds each network attempt separately from the whole-call deadline.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub attempt_timeout_ms: Option<u64>,
}
