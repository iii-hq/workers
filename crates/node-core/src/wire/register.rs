use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct RegisterRequest {
    /// e.g. `my-app::greet`. The segment before the first `::` is the
    /// namespace; the first registration claims it and later ids must share
    /// it. node-engine keeps one runtime per namespace as an implementation
    /// detail you never see or manage.
    pub function_id: String,
    /// JavaScript defining `handler(payload)`:
    /// `export function handler(p) { return p.n * 2 }`. The runtime loads
    /// the source, calls `handler`, and JSON-serialises the return value.
    pub source: String,
    /// What `engine::functions::info` shows a caller — write one.
    #[serde(default)]
    pub description: Option<String>,
}

// `source` is tenant-authored code — a secret under the same rule the old
// `FunctionDef` hand-rolled Debug existed for. This is where it enters the
// crate, and nothing downstream retains a copy, so this is the only place it
// needs redacting.
impl std::fmt::Debug for RegisterRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterRequest")
            .field("function_id", &self.function_id)
            .field("source", &"<redacted>")
            .field("description", &self.description)
            .finish()
    }
}

// No secrets here — the id and namespace are public on the bus — so `Debug`
// derives.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisterResponse {
    /// The id now answering on the bus.
    pub function_id: String,
    /// The namespace it was registered under, normalised to end in `::`.
    pub namespace: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The finding this guards: a derived `Debug` on any type carrying
    /// tenant-authored source prints it the moment anything formats it with
    /// `{:?}`.
    #[test]
    fn debug_redacts_source_only() {
        let req = RegisterRequest {
            function_id: "app::greet".into(),
            source: "SECRET_TENANT_SOURCE_1234".into(),
            description: Some("greets".into()),
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("SECRET_TENANT_SOURCE_1234"),
            "leaked tenant source: {rendered}"
        );
        assert!(
            rendered.contains("app::greet"),
            "non-secret fields should still show: {rendered}"
        );
        assert!(
            rendered.contains("greets"),
            "non-secret fields should still show: {rendered}"
        );
    }
}
