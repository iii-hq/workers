//! llm-router entry schema composition (spec § router::provider::register).
use std::collections::BTreeMap;

use crate::settings::MAX_RETRY_MAX;
use crate::types::errors::{RouterCode, RouterError};
use serde_json::{json, Value};

pub fn default_provider_schema(defaults: &Value) -> Value {
    let mut api_url = json!({ "type": "string" });
    if let Some(u) = defaults.get("api_url").and_then(Value::as_str) {
        api_url["default"] = json!(u);
    }
    let mut max_tokens = json!({ "type": "number" });
    if let Some(m) = defaults.get("max_tokens").and_then(Value::as_u64) {
        max_tokens["default"] = json!(m);
    }
    json!({
        "type": "object",
        "additionalProperties": true,
        "properties": {
            "api_key": { "type": "string", "writeOnly": true, "format": "password" },
            "api_url": api_url,
            "max_tokens": max_tokens,
        }
    })
}

/// Custom schemas MUST mark secret-bearing fields write-only (spec § register).
pub fn validate_custom_schema(schema: &Value) -> Result<(), RouterError> {
    let secretish = regex::Regex::new("(?i)(key|token|secret|password)").expect("static regex");
    if let Some(Value::Object(props)) = schema.get("properties") {
        for (field, def) in props {
            if secretish.is_match(field) && def.get("writeOnly") != Some(&Value::Bool(true)) {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("config_schema field \"{field}\" looks secret-bearing and must set writeOnly: true"),
                ));
            }
        }
    }
    Ok(())
}

/// Build one provider's config slice schema: its custom `config_schema` or
/// the default `{api_key, api_url, max_tokens}`.
pub fn provider_entry_schema(config_schema: Option<&Value>, defaults: &Value) -> Value {
    config_schema
        .cloned()
        .unwrap_or_else(|| default_provider_schema(defaults))
}

/// Router-owned fields + per-provider slices. `null` is admitted because a
/// freshly registered entry holds null until the operator first writes —
/// the engine validates the existing value against the schema on re-register.
pub fn compose_entry_schema(provider_schemas: &BTreeMap<String, Value>) -> Value {
    json!({
        "type": ["object", "null"],
        "additionalProperties": false,
        "properties": {
            "default_provider": { "type": "string" },
            "routing_heuristics": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["pattern", "provider"],
                    "properties": { "pattern": { "type": "string" }, "provider": { "type": "string" } }
                }
            },
            "settings": {
                "type": "object",
                "properties": {
                    "stream_timeout_ms": { "type": "number", "default": 300000 },
                    "idle_timeout_ms": { "type": "number", "default": 120000 },
                    "retry_max": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MAX_RETRY_MAX,
                        "default": 2
                    },
                    "output_token_max": { "type": "number", "default": 32000 }
                }
            },
            "providers": { "type": "object", "properties": provider_schemas }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_provider_schema_marks_api_key_write_only() {
        let s = default_provider_schema(&json!({ "api_url": "https://x", "max_tokens": 8192 }));
        assert_eq!(s["properties"]["api_key"]["writeOnly"], true);
        assert_eq!(s["properties"]["api_key"]["format"], "password");
        assert_eq!(s["properties"]["api_url"]["default"], "https://x");
        assert_eq!(s["properties"]["max_tokens"]["default"], 8192);
    }

    #[test]
    fn compose_nests_provider_slices_plus_router_owned_fields() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert("anthropic".to_string(), default_provider_schema(&json!({})));
        let s = compose_entry_schema(&providers);
        for key in [
            "default_provider",
            "providers",
            "routing_heuristics",
            "settings",
        ] {
            assert!(s["properties"].get(key).is_some(), "missing {key}");
        }
        assert!(s["properties"]["providers"]["properties"]
            .get("anthropic")
            .is_some());
    }

    #[test]
    fn provider_entry_schema_does_not_add_a_system_prompt_field() {
        let s = provider_entry_schema(None, &json!({}));
        assert!(s["properties"].get("system_prompt").is_none());
    }

    #[test]
    fn compose_preserves_provider_schemas_without_system_prompt_fields() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert("anthropic".to_string(), default_provider_schema(&json!({})));
        providers.insert(
            "custom".to_string(),
            json!({ "type": "object", "properties": { "region": { "type": "string" } } }),
        );
        let s = compose_entry_schema(&providers);
        for id in ["anthropic", "custom"] {
            let slice = &s["properties"]["providers"]["properties"][id]["properties"];
            assert!(slice.get("system_prompt").is_none(), "{id}");
        }
        // pre-existing fields survive
        assert_eq!(
            s["properties"]["providers"]["properties"]["custom"]["properties"]["region"]["type"],
            "string"
        );
    }

    #[test]
    fn custom_schemas_must_mark_secretish_fields_write_only() {
        let bad = json!({ "type": "object", "properties": { "api_key": { "type": "string" } } });
        assert!(validate_custom_schema(&bad).is_err());
        let good = json!({ "type": "object", "properties": { "api_key": { "type": "string", "writeOnly": true } } });
        assert!(validate_custom_schema(&good).is_ok());
        assert!(validate_custom_schema(&json!({ "type": "object" })).is_ok());
    }

    #[test]
    fn retry_setting_is_an_operationally_bounded_integer() {
        let schema = compose_entry_schema(&BTreeMap::new());
        let retry = &schema["properties"]["settings"]["properties"]["retry_max"];
        assert_eq!(retry["type"], "integer");
        assert_eq!(retry["minimum"], 0);
        assert_eq!(retry["maximum"], MAX_RETRY_MAX);
    }
}
