use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::Lang;

/// `list_runtimes` takes no arguments; this empty struct publishes an
/// accurate (empty-object) request schema.
///
/// NOTE: deliberately NOT `#[serde(deny_unknown_fields)]`. The engine
/// injects a `_caller_worker_id` field into every call's payload, so a
/// strict no-arg struct would reject EVERY call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRuntimesRequest {}

/// One live runtime, exactly as this worker tracks it.
#[derive(Serialize, JsonSchema)]
pub struct RuntimeSummary {
    /// Treat as a secret: it is the capability to run into or tear down
    /// this runtime (`sandbox-code-runner::run` / `teardown`).
    pub runtime_id: String,
    pub lang: Lang,
    /// The backing iii-sandbox VM's id.
    pub sandbox_id: String,
    /// Wall-clock creation time, epoch milliseconds.
    pub created_at_ms: u64,
    /// Bus function ids registered onto this runtime, in registration
    /// order. Always empty for a kept-run runtime — nothing is ever
    /// registered onto one.
    pub registered_functions: Vec<String>,
    /// `true` when the backing VM no longer appears in `sandbox::list` —
    /// the daemon's idle reaper (or an out-of-band stop) took it and this
    /// worker has not yet observed the loss through a failed use. Absent
    /// when the reconciliation read itself failed, so absence means
    /// "unknown", never "alive".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm_gone: Option<bool>,
}

// `runtime_id` is a capability and `sandbox_id` must never reach logs.
// Hand-rolled `Debug` keeps both out of `{:?}`.
impl std::fmt::Debug for RuntimeSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeSummary")
            .field("runtime_id", &"<redacted>")
            .field("lang", &self.lang)
            .field("sandbox_id", &"<redacted>")
            .field("created_at_ms", &self.created_at_ms)
            .field("registered_functions", &self.registered_functions)
            .finish()
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListRuntimesResponse {
    /// Every live runtime — kept-run and namespace runtimes alike — newest
    /// first. One-shot run VMs never appear: they are destroyed before
    /// their run's response is sent.
    pub runtimes: Vec<RuntimeSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_tolerates_unknown_fields() {
        serde_json::from_value::<ListRuntimesRequest>(
            serde_json::json!({ "_caller_worker_id": "w1", "stray": true }),
        )
        .expect("unknown fields are ignored, not refused");
    }

    #[test]
    fn debug_redacts_the_runtime_and_sandbox_ids() {
        let s = RuntimeSummary {
            runtime_id: "rt-secret-capability".into(),
            lang: Lang::Node,
            sandbox_id: "sb-secret".into(),
            created_at_ms: 1,
            registered_functions: vec!["app::greet".into()],
            vm_gone: None,
        };
        let rendered = format!("{s:?}");
        assert!(!rendered.contains("rt-secret-capability"), "{rendered}");
        assert!(!rendered.contains("sb-secret"), "{rendered}");
        assert!(rendered.contains("app::greet"), "{rendered}");
        assert!(rendered.contains("Node"), "{rendered}");
    }
}
