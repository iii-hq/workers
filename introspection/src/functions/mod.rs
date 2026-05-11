pub mod context;
pub mod functions_mod;
pub mod registry;
pub mod stream;
pub mod workers;

/// Namespace prefixes filtered out of `introspection::functions::list` by
/// default. Each one is noise for an agent deciding what to call — they
/// belong to telemetry, internal probes, or skill/resource plumbing the
/// agent has no reason to invoke directly.
pub const DEFAULT_EXCLUDED_NAMESPACES: &[&str] = &[
    "skills::resources-",
    "skills::register",
    "skills::unregister",
    "skills::fetch_skill",
    "skill::register",
    "skill::unregister",
    "engine::workers::register",
    "engine::console::",
    "engine::telemetry::",
    "telemetry::",
    "iii-telemetry::",
    "iii-observability::",
    "hook-fanout::",
    "policy-denylist::",
    "auth::",
    "introspection::stream::",
];

/// Known engine-builtin workers (NOT separate iii-sdk worker processes).
/// They show up in `engine::workers::list` as `available` until the engine
/// config enables them via a config block.
pub const ENGINE_BUILTINS: &[(&str, &str)] = &[
    (
        "iii-sandbox",
        "Engine builtin. Enable by adding `iii-sandbox: { runtime: libkrun | docker, image: python, idle_timeout_secs: 300 }` to your engine config.yaml, then restart engine with `--config <path>` instead of `--use-default-config`.",
    ),
    (
        "iii-http",
        "Engine builtin HTTP trigger surface. Enabled via `iii-http:` config block.",
    ),
    (
        "iii-cron",
        "Engine builtin cron trigger surface. Enabled via `iii-cron:` config block.",
    ),
    (
        "iii-pubsub",
        "Engine builtin pub/sub topic surface. Always available.",
    ),
    (
        "iii-state",
        "Engine builtin key-value state. Always available.",
    ),
    (
        "iii-stream",
        "Engine builtin append-only streams. Always available.",
    ),
    (
        "iii-queue",
        "Engine builtin durable queues. Always available.",
    ),
    (
        "iii-engine-functions",
        "Engine builtin introspection (engine::functions::list etc).",
    ),
    (
        "iii-worker-manager",
        "Engine builtin worker lifecycle. `iii worker add` routes through here.",
    ),
    (
        "iii-console",
        "Engine builtin web console + queue publishers.",
    ),
];

use std::sync::Arc;

use iii_sdk::{protocol::TriggerRequest, IIIError, III};
use serde_json::Value;

pub async fn call(iii: &Arc<III>, function_id: &str, payload: Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

pub fn is_excluded(fn_id: &str, extra: &[String]) -> bool {
    DEFAULT_EXCLUDED_NAMESPACES
        .iter()
        .any(|p| fn_id.starts_with(p))
        || extra.iter().any(|p| fn_id.starts_with(p))
}

pub fn builtin_hint(worker_name: &str) -> Option<&'static str> {
    ENGINE_BUILTINS
        .iter()
        .find(|(n, _)| *n == worker_name)
        .map(|(_, h)| *h)
}

/// Effective function count for a worker entry: prefer the explicit
/// `function_count` field if present, otherwise derive from the
/// embedded `functions` array.
pub fn effective_fn_count(w: &Value) -> u64 {
    w.get("function_count")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            w.get("functions")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(0)
}

/// Engine fallback names are `<hostname>:<port>` — drop them from agent-
/// facing surfaces because they're transient debug connections.
pub fn is_anonymous_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if let Some(pos) = name.rfind(':') {
        let after = &name[pos + 1..];
        if !after.is_empty()
            && after.chars().all(|c| c.is_ascii_digit())
            && (pos == 0 || bytes[pos - 1] != b':')
        {
            return true;
        }
    }
    false
}

/// Worker rows in `engine::workers::list` can include multiple entries for
/// the same logical worker (stale reconnects). Dedup by `name`, preferring
/// `status == connected` over anything else and the row with the highest
/// `function_count` within the same status class. Anonymous `host:port`
/// names are dropped entirely.
pub fn dedup_workers(workers: Vec<Value>) -> Vec<Value> {
    let mut map: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for w in workers {
        let name = w
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || is_anonymous_name(&name) {
            continue;
        }
        let status = w.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let fn_count = effective_fn_count(&w);
        let new_score = if status == "connected" { 1_000_000 } else { 0 } + fn_count;
        match map.get(&name) {
            None => {
                map.insert(name, w);
            }
            Some(prev) => {
                let prev_status = prev.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let prev_count = effective_fn_count(prev);
                let prev_score = if prev_status == "connected" {
                    1_000_000
                } else {
                    0
                } + prev_count;
                if new_score > prev_score {
                    map.insert(name, w);
                }
            }
        }
    }
    let mut out: Vec<Value> = map.into_values().collect();
    out.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anonymous_names_match_host_port_fallback() {
        assert!(is_anonymous_name("MacBookPro.local:35878"));
        assert!(is_anonymous_name("worker.host:1"));
        assert!(!is_anonymous_name("shell-bash"));
        assert!(!is_anonymous_name("introspection::workers::list"));
        assert!(!is_anonymous_name(""));
    }

    #[test]
    fn effective_fn_count_prefers_explicit_then_array_len() {
        assert_eq!(
            effective_fn_count(&json!({"function_count": 7, "functions": [1,2,3]})),
            7
        );
        assert_eq!(effective_fn_count(&json!({"functions": [1, 2, 3]})), 3);
        assert_eq!(effective_fn_count(&json!({})), 0);
    }

    #[test]
    fn is_excluded_drops_known_noise_prefixes() {
        assert!(is_excluded("skills::resources-list", &[]));
        assert!(is_excluded("telemetry::flush", &[]));
        assert!(!is_excluded("shell::bash::exec", &[]));
        assert!(is_excluded("custom::noisy", &["custom::".to_string()]));
    }

    #[test]
    fn dedup_workers_drops_anon_and_prefers_connected_and_higher_fn_count() {
        let input = vec![
            json!({"name": "MacBookPro:1234", "status": "connected", "function_count": 5}),
            json!({"name": "shell-bash", "status": "disconnected", "function_count": 3}),
            json!({"name": "shell-bash", "status": "connected", "function_count": 3}),
            json!({"name": "introspection", "status": "connected", "function_count": 2}),
            json!({"name": "introspection", "status": "connected", "function_count": 8}),
        ];
        let out = dedup_workers(input);
        // anon dropped
        assert!(!out.iter().any(|w| w["name"] == "MacBookPro:1234"));
        // one row per logical worker
        assert_eq!(out.len(), 2);
        // shell-bash kept the connected row
        let shell = out.iter().find(|w| w["name"] == "shell-bash").unwrap();
        assert_eq!(shell["status"], "connected");
        // introspection kept the higher fn count
        let intro = out.iter().find(|w| w["name"] == "introspection").unwrap();
        assert_eq!(intro["function_count"], 8);
    }

    #[test]
    fn builtin_hint_resolves_known_engine_builtins() {
        assert!(builtin_hint("iii-sandbox").is_some());
        assert!(builtin_hint("iii-http").is_some());
        assert!(builtin_hint("unknown-worker").is_none());
    }
}
