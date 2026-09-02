use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lang::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct RegisterRequest {
    /// Bus id, e.g. `my-app::greet`; the segment before the first `::` is the namespace,
    /// which holds one persistent runtime per lang.
    pub function_id: String,
    /// Source that defines `handler(payload)` in `lang`: `export function handler(payload)
    /// {…}` (node) or `def handler(payload): …` (python).
    // Node publishes itself via `iii.registerFunction`; python's source just
    // defines the name and the host publishes it. Anything else the source
    // defines persists across invocations: the namespace runs on one pinned
    // interpreter.
    pub source: String,
    /// What `engine::functions::info` shows a caller — write one.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional JSON Schema object (≤ 16 KiB, must constrain: `type`/`properties`/`$ref`)
    /// for the payload; shown by `engine::functions::info`, not enforced.
    #[serde(default)]
    pub request_format: Option<serde_json::Value>,
    /// Optional JSON Schema for the handler's return value; same rules as `request_format`.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// Language of the namespace runtime this handler runs in.
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
