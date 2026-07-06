//! llm-router entry schema composition (spec § router::provider::register).
use std::collections::BTreeMap;

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

/// The per-slice `system_prompt` knob. Nullable on purpose: the console
/// renders `["string","null"]` as a set/unset toggle and only shows the
/// editor when set. `format: "textarea"` picks the multi-line pill editor.
/// When a provider declared a prompt, it rides in `default` so the console
/// pre-fills the editor with it (an editable starting point) the first time
/// an operator flips the field to "set".
pub fn system_prompt_schema(default: Option<&str>) -> Value {
    let mut schema = json!({
        "type": ["string", "null"],
        "format": "textarea",
        "description": "Override the provider-declared identity prompt; unset serves the provider's default."
    });
    if let Some(d) = default {
        schema["default"] = json!(d);
    }
    schema
}

/// Build one provider's config slice schema: its custom `config_schema` (or
/// the default `{api_key, api_url, max_tokens}`) plus the `system_prompt`
/// knob carrying the provider-declared prompt as its default.
pub fn provider_entry_schema(
    config_schema: Option<&Value>,
    defaults: &Value,
    declared_prompt: Option<&str>,
) -> Value {
    let mut schema = config_schema
        .cloned()
        .unwrap_or_else(|| default_provider_schema(defaults));
    if declared_prompt.is_some() {
        if let Some(Value::Object(props)) = schema.get_mut("properties") {
            props.insert(
                "system_prompt".to_string(),
                system_prompt_schema(declared_prompt),
            );
        }
    }
    schema
}

/// Injects the `system_prompt` knob into every slice that doesn't already
/// carry it — providers with a custom `config_schema` (and providers that
/// declared no prompt) get it here; `provider_entry_schema` pre-seeds it for
/// providers that declared one, so `or_insert` preserves that default.
fn with_prompt_fields(schema: &Value) -> Value {
    let mut schema = schema.clone();
    if let Some(Value::Object(props)) = schema.get_mut("properties") {
        props
            .entry("system_prompt")
            .or_insert_with(|| system_prompt_schema(None));
    }
    schema
}

/// Router-owned fields + per-provider slices. `null` is admitted because a
/// freshly registered entry holds null until the operator first writes —
/// the engine validates the existing value against the schema on re-register.
pub fn compose_entry_schema(provider_schemas: &BTreeMap<String, Value>) -> Value {
    let provider_schemas: BTreeMap<&String, Value> = provider_schemas
        .iter()
        .map(|(id, schema)| (id, with_prompt_fields(schema)))
        .collect();
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
                    "retry_max": { "type": "number", "default": 2 },
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
    fn provider_entry_schema_carries_declared_prompt_as_default() {
        // declared prompt → system_prompt.default so the console pre-fills it
        let s = provider_entry_schema(None, &json!({}), Some("DECLARED"));
        let sp = &s["properties"]["system_prompt"];
        assert_eq!(sp["type"], json!(["string", "null"]));
        assert_eq!(sp["format"], "textarea");
        assert_eq!(sp["default"], "DECLARED");
        // no declared prompt → compose still adds the knob, without a default
        let s = provider_entry_schema(None, &json!({}), None);
        assert!(s["properties"].get("system_prompt").is_none());
        let composed = with_prompt_fields(&s);
        assert!(composed["properties"]["system_prompt"]
            .get("default")
            .is_none());
    }

    #[test]
    fn compose_injects_prompt_field_into_every_slice_including_custom() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert("anthropic".to_string(), default_provider_schema(&json!({})));
        providers.insert(
            "custom".to_string(),
            json!({ "type": "object", "properties": { "region": { "type": "string" } } }),
        );
        let s = compose_entry_schema(&providers);
        for id in ["anthropic", "custom"] {
            let slice = &s["properties"]["providers"]["properties"][id]["properties"];
            // nullable so the console renders a set/unset toggle
            assert_eq!(
                slice["system_prompt"]["type"],
                json!(["string", "null"]),
                "{id}"
            );
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
}
