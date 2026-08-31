use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Exactly one of `runtime_id` (a runtime you got back from
/// `sandbox-code-runner::run keep=true`) or `namespace` (a `register_function`
/// namespace, e.g. "app" for ids like `app::greet`) must be set — never both,
/// never neither.
#[derive(Deserialize, JsonSchema)]
pub struct TeardownRequest {
    #[serde(default)]
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
}

impl std::fmt::Debug for TeardownRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeardownRequest")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("namespace", &self.namespace)
            .finish()
    }
}

#[derive(Serialize, JsonSchema)]
pub struct TeardownResponse {
    /// Set when this teardown was addressed by `runtime_id` (never present
    /// alongside `namespace`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// Set when this teardown was addressed by `namespace` — echoes the
    /// namespace, since more than one runtime (one per language) can back
    /// it and there is no single `runtime_id` to report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub torn_down: bool,
    /// Bus function ids this teardown unregistered, across every runtime it
    /// destroyed.
    pub unregistered: Vec<String>,
}

impl std::fmt::Debug for TeardownResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeardownResponse")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("namespace", &self.namespace)
            .field("torn_down", &self.torn_down)
            .field("unregistered", &self.unregistered)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_runtime_id_both_ways() {
        let req = TeardownRequest {
            runtime_id: Some("rt-secret".into()),
            namespace: None,
        };
        assert!(!format!("{req:?}").contains("rt-secret"));
        let res = TeardownResponse {
            runtime_id: Some("rt-secret".into()),
            namespace: None,
            torn_down: true,
            unregistered: vec!["app::x".into()],
        };
        let rendered = format!("{res:?}");
        assert!(!rendered.contains("rt-secret"), "{rendered}");
        assert!(rendered.contains("app::x"), "{rendered}");
    }

    #[test]
    fn wire_omits_absent_addressing_field() {
        let by_id = TeardownResponse {
            runtime_id: Some("rt-1".into()),
            namespace: None,
            torn_down: true,
            unregistered: vec![],
        };
        let v = serde_json::to_value(&by_id).unwrap();
        assert!(!v.as_object().unwrap().contains_key("namespace"));

        let by_ns = TeardownResponse {
            runtime_id: None,
            namespace: Some("app::".into()),
            torn_down: true,
            unregistered: vec![],
        };
        let v = serde_json::to_value(&by_ns).unwrap();
        assert!(!v.as_object().unwrap().contains_key("runtime_id"));
    }
}
