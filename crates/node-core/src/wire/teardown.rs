use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct TeardownRequest {
    /// The runtime to destroy. Unregisters its functions and kills the
    /// isolate. Pass exactly one of this or `namespace`.
    #[serde(default)]
    pub runtime_id: Option<String>,
    /// Destroy the runtime backing this namespace's registered functions.
    /// The only way to reclaim one: `register_function` never returns its
    /// runtime id.
    #[serde(default)]
    pub namespace: Option<String>,
}

// `runtime_id` is a capability — same rule as `RunRequest` and `IdRegistry`.
// `namespace` is not: it is caller-chosen and already shown unredacted
// elsewhere (e.g. `register_function`'s response). Nothing formats this
// today; a derived `Debug` prints it in full the first time something does.
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

#[derive(Debug, Serialize, JsonSchema)]
pub struct TeardownResponse {
    /// Engine function ids removed by this teardown.
    pub unregistered: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_the_runtime_id() {
        let req = TeardownRequest {
            runtime_id: Some("rt-secret-capability".into()),
            namespace: None,
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("rt-secret-capability"),
            "leaked the runtime_id: {rendered}"
        );
        assert!(
            rendered.contains("TeardownRequest"),
            "should still name the type: {rendered}"
        );
    }

    #[test]
    fn debug_shows_the_namespace_unredacted() {
        let req = TeardownRequest {
            runtime_id: None,
            namespace: Some("app::".into()),
        };
        let rendered = format!("{req:?}");
        assert!(
            rendered.contains("app::"),
            "namespace is not a capability and should render plainly: {rendered}"
        );
    }
}
