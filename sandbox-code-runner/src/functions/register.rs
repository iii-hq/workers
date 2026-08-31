use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct RegisterRequest {
    /// e.g. "my-app::greet". The first registration in a namespace (the
    /// segment before `::`) claims it; later ids must share it.
    /// sandbox-code-runner keeps ONE persistent runtime per (namespace, lang)
    /// — the first
    /// registration creates it, later ones reuse it — as an implementation
    /// detail you never see or manage.
    pub function_id: String,
    /// Source that DEFINES `handler(payload)` in `lang`:
    /// `export function handler(payload) {…}` (node) or
    /// `def handler(payload): …` (python). The runner loads the file, calls
    /// `handler`, and JSON-serializes the return value.
    pub source: String,
    /// What engine::functions::info shows a caller — write one.
    #[serde(default)]
    pub description: Option<String>,
    /// Which runner backs this namespace: "node" or "python". A namespace's
    /// language is fixed by its first registration; a later id under the
    /// same namespace but a different lang is refused.
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
