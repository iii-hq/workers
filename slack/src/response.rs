//! Typed response wrapper for Slack Web API calls.
//!
//! Slack methods return `{ "ok": true, ... }` with method-specific fields. We
//! surface that as a typed object (`ok` plus the rest as data) so every
//! registered `slack::*` function publishes a typed response schema — never the
//! permissive AnyValue schema a bare `serde_json::Value` handler would emit
//! (see `docs/sops/new-worker.md` §5 typed-schema rule).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SlackResponse {
    /// Slack's success flag (always true here; failures surface as errors).
    pub ok: bool,
    /// The remaining method-specific fields of the Slack response payload.
    #[serde(flatten)]
    pub data: Map<String, Value>,
}

impl SlackResponse {
    /// Wrap a raw Slack payload `Value` into the typed shape.
    pub fn from_value(value: Value) -> Self {
        let mut map = match value {
            Value::Object(m) => m,
            other => {
                let mut m = Map::new();
                m.insert("result".into(), other);
                m
            }
        };
        let ok = map.remove("ok").and_then(|v| v.as_bool()).unwrap_or(true);
        SlackResponse { ok, data: map }
    }
}
