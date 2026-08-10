use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lang::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct RegisterRequest {
    /// e.g. `my-app::greet`. The segment before the first `::` is the
    /// namespace; the first registration claims it for a language and later
    /// ids must share both.
    pub function_id: String,
    /// Source that DEFINES `handler(payload)` in `lang`.
    ///
    /// Node publishes itself, by calling `iii.registerFunction`. Python's
    /// source just defines the name — the host publishes it — and anything
    /// else the source defines stays available to every later invocation,
    /// because the namespace runs on one pinned interpreter.
    pub source: String,
    /// What `engine::functions::info` shows a caller — write one.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional JSON Schema for the payload the registered function expects —
    /// `engine::functions::info` then shows it in place of "any". Must be a
    /// JSON object that actually constrains something (`type`, `properties`,
    /// `$ref`, …); at most 16 KiB serialized.
    #[serde(default)]
    pub request_format: Option<serde_json::Value>,
    /// Optional JSON Schema for the value the handler returns; same rules as
    /// `request_format`.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// Which engine backs this namespace. Required here, unlike on `run`: a
    /// registration has no runtime to infer it from.
    pub lang: Lang,
}

// `source` is the tenant's own program.
impl std::fmt::Debug for RegisterRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterRequest")
            .field("function_id", &self.function_id)
            .field("source", &"<redacted>")
            .field("description", &self.description)
            .field("request_format", &self.request_format)
            .field("response_format", &self.response_format)
            .field("lang", &self.lang)
            .finish()
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisterResponse {
    pub function_id: String,
    pub registered: bool,
}
