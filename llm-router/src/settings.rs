//! Router settings from the llm-router configuration entry (engine
//! `configuration::*` iii functions — spec § Configuration, not env vars).
//! Never panics: a malformed entry never takes the router down.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::routing::Heuristic;
use crate::types::errors::{RouterCode, RouterError};

#[derive(Debug, Clone, PartialEq)]
pub struct RouterSettings {
    pub default_provider: Option<String>,
    pub routing_heuristics: Vec<Heuristic>,
    pub stream_timeout_ms: u64, // total budget for provider::<id>::stream (spec: 300s)
    pub idle_timeout_ms: u64,   // max gap between frames, ping included (spec: 120s)
    pub retry_max: u32,         // further attempts after the first (spec: 2)
    pub output_token_max: u64,  // soft cap (spec: 32_000)
}

impl Default for RouterSettings {
    fn default() -> Self {
        RouterSettings {
            default_provider: None,
            routing_heuristics: vec![],
            stream_timeout_ms: 300_000,
            idle_timeout_ms: 120_000,
            retry_max: 2,
            output_token_max: 32_000,
        }
    }
}

fn pos_u64(v: Option<&Value>, fallback: u64) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(fallback)
}

pub fn parse_settings(entry_value: &Value) -> RouterSettings {
    let mut out = RouterSettings::default();
    let Value::Object(root) = entry_value else {
        return out;
    };

    if let Some(p) = root.get("default_provider").and_then(Value::as_str) {
        out.default_provider = Some(p.to_string());
    }
    if let Some(Value::Array(items)) = root.get("routing_heuristics") {
        for h in items {
            if let (Some(pattern), Some(provider)) = (
                h.get("pattern").and_then(Value::as_str),
                h.get("provider").and_then(Value::as_str),
            ) {
                out.routing_heuristics.push(Heuristic {
                    pattern: pattern.into(),
                    provider: provider.into(),
                });
            }
        }
    }
    if let Some(Value::Object(s)) = root.get("settings") {
        out.stream_timeout_ms = pos_u64(s.get("stream_timeout_ms"), out.stream_timeout_ms);
        out.idle_timeout_ms = pos_u64(s.get("idle_timeout_ms"), out.idle_timeout_ms);
        out.retry_max = pos_u64(s.get("retry_max"), out.retry_max as u64) as u32;
        out.output_token_max = pos_u64(s.get("output_token_max"), out.output_token_max);
    }
    out
}

/// Semantic validation that JSON Schema cannot express (cross-field limits,
/// regex compilation, live provider references and absolute HTTP URLs).
/// Callers must run this before swapping the active snapshot.
pub fn validate_settings(
    entry_value: &Value,
    registered_providers: &[String],
) -> Result<(), RouterError> {
    if entry_value.is_null() {
        return Ok(());
    }
    let root = entry_value.as_object().ok_or_else(|| {
        RouterError::new(
            RouterCode::InvalidRequest,
            "configuration must be an object or null",
        )
    })?;
    let registered = |id: &str| registered_providers.iter().any(|p| p == id);

    if let Some(default) = root.get("default_provider") {
        let id = default
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                RouterError::new(
                    RouterCode::InvalidRequest,
                    "default_provider must be a non-empty string",
                )
            })?;
        if !registered(id) {
            return Err(RouterError::new(
                RouterCode::InvalidRequest,
                format!("default_provider references unknown provider {id}"),
            ));
        }
    }

    if let Some(heuristics) = root.get("routing_heuristics") {
        let rows = heuristics.as_array().ok_or_else(|| {
            RouterError::new(
                RouterCode::InvalidRequest,
                "routing_heuristics must be an array",
            )
        })?;
        for (index, row) in rows.iter().enumerate() {
            let row = row.as_object().ok_or_else(|| {
                RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("routing_heuristics[{index}] must be an object"),
                )
            })?;
            let pattern = row.get("pattern").and_then(Value::as_str).ok_or_else(|| {
                RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("routing_heuristics[{index}].pattern must be a string"),
                )
            })?;
            regex::Regex::new(pattern).map_err(|e| {
                RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("routing_heuristics[{index}].pattern is invalid: {e}"),
                )
            })?;
            let provider = row.get("provider").and_then(Value::as_str).ok_or_else(|| {
                RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("routing_heuristics[{index}].provider must be a string"),
                )
            })?;
            if !registered(provider) {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!(
                        "routing_heuristics[{index}].provider references unknown provider {provider}"
                    ),
                ));
            }
        }
    }

    let parsed = parse_settings(entry_value);
    if let Some(settings) = root.get("settings") {
        let settings = settings.as_object().ok_or_else(|| {
            RouterError::new(RouterCode::InvalidRequest, "settings must be an object")
        })?;
        for (name, allows_zero, maximum) in [
            ("stream_timeout_ms", false, None),
            ("idle_timeout_ms", false, None),
            ("retry_max", true, Some(10)),
            ("output_token_max", false, None),
        ] {
            if let Some(value) = settings.get(name) {
                let number = value.as_u64().ok_or_else(|| {
                    RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("settings.{name} must be an integer"),
                    )
                })?;
                if !allows_zero && number == 0 {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("settings.{name} must be greater than zero"),
                    ));
                }
                if maximum.is_some_and(|max| number > max) {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("settings.{name} must be at most {}", maximum.unwrap()),
                    ));
                }
            }
        }
    }
    if parsed.idle_timeout_ms > parsed.stream_timeout_ms {
        return Err(RouterError::new(
            RouterCode::InvalidRequest,
            "settings.idle_timeout_ms must not exceed settings.stream_timeout_ms",
        ));
    }

    if let Some(Value::Object(providers)) = root.get("providers") {
        for (id, slice) in providers {
            let Some(slice) = slice.as_object() else {
                continue;
            };
            if let Some(api_url) = slice.get("api_url") {
                let raw = api_url.as_str().ok_or_else(|| {
                    RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("providers.{id}.api_url must be a string"),
                    )
                })?;
                let parsed = url::Url::parse(raw).map_err(|e| {
                    RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("providers.{id}.api_url must be an absolute URL: {e}"),
                    )
                })?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("providers.{id}.api_url must use http or https"),
                    ));
                }
            }
            if let Some(max_tokens) = slice.get("max_tokens") {
                if max_tokens.as_u64().filter(|v| *v > 0).is_none() {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        format!("providers.{id}.max_tokens must be a positive integer"),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Provider config slices from the entry value ({ providers: { id: slice } }).
pub fn provider_slices(entry_value: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(providers)) = entry_value.get("providers") {
        for (id, slice) in providers {
            if slice.is_object() {
                out.insert(id.clone(), slice.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_for_null_or_garbage() {
        assert_eq!(
            parse_settings(&serde_json::Value::Null),
            RouterSettings::default()
        );
        assert_eq!(parse_settings(&json!("nope")), RouterSettings::default());
        assert_eq!(
            parse_settings(&json!({ "settings": "nope" })),
            RouterSettings::default()
        );
    }

    #[test]
    fn reads_default_provider_heuristics_and_overrides() {
        let parsed = parse_settings(&json!({
            "default_provider": "anthropic",
            "routing_heuristics": [{ "pattern": "^gpt-", "provider": "openai" }, { "bad": true }],
            "settings": { "stream_timeout_ms": 60000, "retry_max": 0 }
        }));
        assert_eq!(parsed.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(parsed.routing_heuristics.len(), 1);
        assert_eq!(parsed.stream_timeout_ms, 60_000);
        assert_eq!(parsed.retry_max, 0);
        assert_eq!(
            parsed.idle_timeout_ms,
            RouterSettings::default().idle_timeout_ms
        );
    }

    #[test]
    fn provider_slices_extracts_object_slices_only() {
        let slices = provider_slices(&json!({ "providers": { "a": { "k": 1 }, "bad": 7 } }));
        assert_eq!(slices.len(), 1);
        assert!(slices.contains_key("a"));
        assert!(provider_slices(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn semantic_validation_rejects_silent_fallbacks() {
        let providers = vec!["openai".to_string()];
        for invalid in [
            json!({ "settings": { "retry_max": 11 } }),
            json!({ "settings": { "stream_timeout_ms": 100, "idle_timeout_ms": 101 } }),
            json!({ "routing_heuristics": [{ "pattern": "([", "provider": "openai" }] }),
            json!({ "routing_heuristics": [{ "pattern": "^gpt", "provider": "missing" }] }),
            json!({ "providers": { "openai": { "api_url": "localhost:1234" } } }),
        ] {
            assert!(
                validate_settings(&invalid, &providers).is_err(),
                "{invalid}"
            );
        }
        assert!(validate_settings(
            &json!({
                "default_provider": "openai",
                "settings": { "stream_timeout_ms": 100, "idle_timeout_ms": 100, "retry_max": 0 },
                "providers": { "openai": { "api_url": "https://api.example.test/v1", "max_tokens": 1 } }
            }),
            &providers,
        )
        .is_ok());
    }
}
