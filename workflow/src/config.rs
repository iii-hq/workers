use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Default wait guard for a parked pending call before the sweep resolves
    /// it with an error. Milliseconds.
    #[serde(default = "default_pending_timeout_ms")]
    pub default_pending_timeout_ms: u64,

    /// Cron expression (6-field) for the pending-call expiry sweep. The one
    /// structural field: a change re-binds the cron trigger live.
    #[serde(default = "default_sweep_expression")]
    pub sweep_expression: String,

    /// RPC timeout for `engine::*` and generic `iii.trigger` dispatch.
    /// Milliseconds.
    #[serde(default = "default_dispatch_timeout_ms")]
    pub dispatch_timeout_ms: u64,

    /// Maximum number of retry attempts per node before the sweep marks it
    /// failed. Hot-applies via config-cell swap (not structural).
    #[serde(default = "default_max_node_retries")]
    pub max_node_retries: u32,
}

fn default_pending_timeout_ms() -> u64 {
    1_800_000
}
fn default_sweep_expression() -> String {
    "0 * * * * *".to_string()
}
fn default_dispatch_timeout_ms() -> u64 {
    30_000
}
fn default_max_node_retries() -> u32 {
    1
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_pending_timeout_ms: default_pending_timeout_ms(),
            sweep_expression: default_sweep_expression(),
            dispatch_timeout_ms: default_dispatch_timeout_ms(),
            max_node_retries: default_max_node_retries(),
        }
    }
}

impl WorkerConfig {
    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker (does NOT re-expand) and tolerant of a zero-field
    /// object (serde defaults fill in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker. Field
    /// doc-comments become property descriptions; the shipped defaults are
    /// attached as a top-level `example`.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_from_empty_object() {
        let cfg = WorkerConfig::from_json(&json!({})).expect("parse ok");
        assert_eq!(cfg, WorkerConfig::default());
        assert_eq!(cfg.default_pending_timeout_ms, 1_800_000);
        assert_eq!(cfg.sweep_expression, "0 * * * * *");
        assert_eq!(cfg.dispatch_timeout_ms, 30_000);
    }
}
