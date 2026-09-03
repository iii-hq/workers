use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct RegisterRequest {
    /// Bus id, e.g. `my-app::greet`; the segment before the first `::` is the namespace,
    /// which holds one persistent runtime per lang.
    pub function_id: String,
    /// Source that defines `handler(payload)` in `lang`: `export function handler(payload)
    /// {…}` (node) or `def handler(payload): …` (python).
    pub source: String,
    /// What `engine::functions::info` shows a caller — write one.
    #[serde(default)]
    pub description: Option<String>,
    /// Language of the namespace runtime this handler runs in.
    pub lang: Lang,
}

// `source` is tenant-authored. `function_id`, `description` and `lang` are
// not secrets, so a derived `Debug` would be fine too — hand-rolled only to
// keep `source` out.
impl std::fmt::Debug for RegisterRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterRequest")
            .field("function_id", &self.function_id)
            .field("source", &"<redacted>")
            .field("description", &self.description)
            .field("lang", &self.lang)
            .finish()
    }
}

// No secrets here — the id is public on the bus — so `Debug` derives. `//`,
// not `///`: this is internal rationale, and schemars would otherwise lift a
// doc comment here into the response schema's `description`, shipping it to
// anyone who calls `engine::functions::info`.
#[derive(Serialize, Debug, JsonSchema)]
pub struct RegisterResponse {
    pub function_id: String,
    pub registered: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_source_only() {
        let req = RegisterRequest {
            function_id: "app::greet".into(),
            source: "SECRET_HANDLER_SOURCE".into(),
            description: Some("greets".into()),
            lang: Lang::Node,
        };
        let rendered = format!("{req:?}");
        assert!(!rendered.contains("SECRET_HANDLER_SOURCE"), "{rendered}");
        assert!(rendered.contains("app::greet"), "{rendered}");
        assert!(rendered.contains("Node"), "{rendered}");
    }
}
