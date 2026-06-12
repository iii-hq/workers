//! Router settings from the llm-router configuration entry (engine
//! `configuration::*` iii functions — spec § Configuration, not env vars).
//! Never panics: a malformed entry never takes the router down.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::routing::Heuristic;

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
}
