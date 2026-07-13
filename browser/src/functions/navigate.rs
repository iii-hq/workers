//! `browser::navigate` — go to a URL and wait for the load to settle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NavigateInput {
    pub session_id: String,
    /// Absolute URL; scheme must be on the configured allowlist.
    pub url: String,
    /// Upper bound on the navigation wait; clamped to `max_timeout_ms`.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NavigateOutput {
    pub ok: bool,
    /// URL after redirects.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// True when the load event did not fire inside the timeout; the page
    /// may still be usable; snapshot to check.
    pub timed_out: bool,
}
