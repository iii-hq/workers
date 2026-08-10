use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Exactly one of `runtime_id` (a kept run's runtime) or `namespace` (a
/// `register_function` namespace) must be set — never both, never neither.
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

#[derive(Debug, Serialize, JsonSchema)]
pub struct TeardownResponse {
    /// Set when this teardown was addressed by `runtime_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// Set when addressed by `namespace` — echoed in its canonical `app::`
    /// form, since more than one runtime (one per language) can back it and
    /// there is no single `runtime_id` to report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub torn_down: bool,
    /// Bus function ids this teardown unregistered, across every runtime it
    /// destroyed.
    pub unregistered: Vec<String>,
}
