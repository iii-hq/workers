//! The worker's one configuration entry — id `approval-gate` — in the
//! engine's built-in `configuration` worker (approval-gate.md §
//! Configuration; same pattern as llm-router). Deployment approval
//! defaults live here and nowhere else. Soft dependency: without the
//! configuration worker the gate runs on built-in defaults — fail-safe,
//! never fail-open.
//!
//! Like llm-router, `configuration::register` is called WITHOUT an
//! `initial_value`, so operator-stored values survive every re-register;
//! the built-in defaults apply in memory whenever the entry value is
//! null or missing.

use std::sync::{Arc, RwLock};

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

use crate::types::PermissionMode;

pub const ENTRY_ID: &str = "approval-gate";

pub const DEFAULT_PENDING_TIMEOUT_MS: i64 = 1_800_000;

/// The parsed `approval-gate` entry value, with built-in defaults for
/// every absent/invalid field.
#[derive(Debug, Clone, PartialEq)]
pub struct GateDefaults {
    /// Effective mode for sessions with no stored settings record.
    pub default_mode: PermissionMode,
    /// Deployment trust profile for auto mode (function ids / globs).
    pub always_allow_seed: Vec<String>,
    /// Hold deadline; drives `expires_at` on pending records.
    pub pending_timeout_ms: i64,
}

impl Default for GateDefaults {
    fn default() -> Self {
        Self {
            default_mode: PermissionMode::Manual,
            always_allow_seed: Vec::new(),
            pending_timeout_ms: DEFAULT_PENDING_TIMEOUT_MS,
        }
    }
}

pub type SharedDefaults = Arc<RwLock<GateDefaults>>;

pub fn shared_defaults() -> SharedDefaults {
    Arc::new(RwLock::new(GateDefaults::default()))
}

pub fn snapshot(defaults: &SharedDefaults) -> GateDefaults {
    defaults
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

pub fn replace(defaults: &SharedDefaults, next: GateDefaults) {
    *defaults
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = next;
}

/// Field-wise tolerant parse: each invalid/absent field falls back to its
/// built-in default. A malformed operator edit can degrade one field,
/// never fail the gate open.
pub fn parse_config_value(value: &Value) -> GateDefaults {
    let built_in = GateDefaults::default();
    let default_mode = value
        .get("default_mode")
        .and_then(|v| serde_json::from_value::<PermissionMode>(v.clone()).ok())
        .unwrap_or(built_in.default_mode);
    let always_allow_seed = value
        .get("always_allow_seed")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or(built_in.always_allow_seed);
    let pending_timeout_ms = value
        .get("pending_timeout_ms")
        .and_then(Value::as_i64)
        .filter(|ms| *ms > 0)
        .unwrap_or(built_in.pending_timeout_ms);
    GateDefaults {
        default_mode,
        always_allow_seed,
        pending_timeout_ms,
    }
}

/// JSON Schema the console renders as the entry's edit form.
pub fn entry_schema() -> Value {
    json!({
        "type": "object",
        "title": "Approval Gate",
        "properties": {
            "default_mode": {
                "type": "string",
                "enum": ["manual", "auto", "full"],
                "default": "manual",
                "description": "Effective permission mode for sessions with no stored approval settings."
            },
            "always_allow_seed": {
                "type": "array",
                "items": { "type": "string" },
                "default": [],
                "description": "Deployment trust profile for auto mode (function ids / globs); copied into a session's settings on its first mutation."
            },
            "pending_timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_PENDING_TIMEOUT_MS,
                "description": "Hold deadline in milliseconds; drives expires_at on pending records."
            }
        },
        "additionalProperties": false
    })
}

pub async fn register_entry(iii: &III) -> Result<(), IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "configuration::register".into(),
        payload: json!({
            "id": ENTRY_ID,
            "name": "Approval Gate",
            "description": "Deployment approval defaults: permission mode for new sessions, the auto-mode trust seed, and the pending-hold timeout.",
            "schema": entry_schema(),
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .map(|_| ())
}

/// Read the entry value; any failure (configuration worker absent, entry
/// unset) yields the built-in defaults.
pub async fn read_defaults(iii: &III) -> GateDefaults {
    match iii
        .trigger(TriggerRequest {
            function_id: "configuration::get".into(),
            payload: json!({ "id": ENTRY_ID }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(reply) => parse_config_value(reply.get("value").unwrap_or(&Value::Null)),
        Err(e) => {
            tracing::info!(error = %e, "configuration worker unavailable; using built-in defaults");
            GateDefaults::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn built_in_defaults_match_the_spec() {
        let d = GateDefaults::default();
        assert_eq!(d.default_mode, PermissionMode::Manual);
        assert!(d.always_allow_seed.is_empty());
        assert_eq!(d.pending_timeout_ms, 1_800_000);
    }

    #[test]
    fn parse_is_field_wise_tolerant() {
        let parsed = parse_config_value(&json!({
            "default_mode": "auto",
            "always_allow_seed": ["state::get", 42, "engine::functions::list"],
            "pending_timeout_ms": "not-a-number"
        }));
        assert_eq!(parsed.default_mode, PermissionMode::Auto);
        // Non-string seed entries are dropped, not fatal.
        assert_eq!(parsed.always_allow_seed.len(), 2);
        assert_eq!(parsed.pending_timeout_ms, DEFAULT_PENDING_TIMEOUT_MS);
    }

    #[test]
    fn parse_of_null_or_garbage_is_all_defaults() {
        assert_eq!(parse_config_value(&Value::Null), GateDefaults::default());
        assert_eq!(parse_config_value(&json!("nope")), GateDefaults::default());
        let zero = parse_config_value(&json!({ "pending_timeout_ms": 0 }));
        assert_eq!(zero.pending_timeout_ms, DEFAULT_PENDING_TIMEOUT_MS);
    }

    #[test]
    fn shared_defaults_replace_and_snapshot() {
        let shared = shared_defaults();
        replace(
            &shared,
            GateDefaults {
                default_mode: PermissionMode::Auto,
                ..GateDefaults::default()
            },
        );
        assert_eq!(snapshot(&shared).default_mode, PermissionMode::Auto);
    }
}
