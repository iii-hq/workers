//! `introspection::context::bootstrap` — compact one-shot context for an
//! agent at session start. Designed to replace the harness's giant skill
//! body dump in the system prompt: < 4KB instead of 50-80KB.
//!
//! Returns:
//!   - connected worker names + brief description
//!   - engine builtins with activation hints (so agent stops trying to
//!     `sandbox::create` when iii-sandbox config block isn't enabled)
//!   - top-level skill index (ids only, no bodies)
//!   - canonical discovery flow ("call introspection::functions::list first")

use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use super::{builtin_hint, ENGINE_BUILTINS};

pub async fn bootstrap(iii: Arc<III>, _payload: Value) -> Result<Value, IIIError> {
    let raw = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;

    let workers = raw
        .get("workers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut connected_map: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();
    let mut live_names: std::collections::HashSet<String> = Default::default();

    for w in workers {
        let name = w
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        let status = w.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if name.is_empty() || is_anonymous_name(&name) {
            continue;
        }
        let live = matches!(status, "connected" | "available");
        if !live {
            continue;
        }
        live_names.insert(name.clone());
        let entry = json!({
            "name": name.clone(),
            "status": status,
            "fn_count": w
                .get("function_count")
                .cloned()
                .or_else(|| {
                    w.get("functions")
                        .and_then(|f| f.as_array())
                        .map(|a| json!(a.len()))
                })
                .unwrap_or(json!(0)),
            "description": w.get("description").cloned(),
        });
        // Dedup: prefer the row with the highest fn_count for the same name.
        match connected_map.get(&name) {
            Some(prev) => {
                let prev_count = prev.get("fn_count").and_then(|v| v.as_u64()).unwrap_or(0);
                let new_count = entry.get("fn_count").and_then(|v| v.as_u64()).unwrap_or(0);
                if new_count > prev_count {
                    connected_map.insert(name, entry);
                }
            }
            None => {
                connected_map.insert(name, entry);
            }
        }
    }
    let mut connected: Vec<Value> = connected_map.into_values().collect();
    connected.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
    });

    // Truly disabled engine builtins: known builtins that are NOT present in
    // workers list at all. Core builtins (iii-state, iii-pubsub, iii-stream,
    // iii-queue, iii-engine-functions, iii-worker-manager, iii-console) are
    // always available so they appear in `connected` above; only iii-sandbox /
    // iii-http / iii-cron (config-block-gated) typically land here.
    let mut not_registered_builtins: Vec<Value> = Vec::new();
    for (n, hint) in ENGINE_BUILTINS {
        if !live_names.contains(*n) {
            not_registered_builtins.push(json!({
                "name": n,
                "status": "not_registered",
                "activation_hint": hint,
            }));
        }
    }

    // Top-level skill index (no bodies).
    let skills = super::call(&iii, "skills::list", json!({}))
        .await
        .ok()
        .and_then(|v| v.get("skills").and_then(|s| s.as_array()).cloned())
        .unwrap_or_default();
    let skill_index: Vec<Value> = skills
        .iter()
        .filter_map(|s| {
            let id = s.get("id").and_then(|v| v.as_str())?;
            // root-level only — `auth-credentials/get_token` etc are children;
            // agent fetches them via skill::fetch when needed.
            if id.contains('/') {
                return None;
            }
            Some(json!({"id": id, "uri": format!("iii://{id}")}))
        })
        .collect();

    // Highlight a few critical hints inline so the agent doesn't have to
    // bootstrap a long discovery sequence on simple asks.
    let pinned_tips = json!([
        "Use `introspection::functions::list { filter: \"<keyword>\" }` for discovery; do NOT call `engine::functions::list` (52KB+).",
        "Once you pick an id, call `introspection::functions::describe { id: \"…\" }` for its schema.",
        "If `iii-sandbox` is missing, you cannot `sandbox::*` — fall back to `shell::bash::exec` or `shell::exec`.",
        "If a worker name appears in `engine_builtins_disabled` here, the engine config doesn't enable it yet; suggest the user re-launch the engine with `--config <yaml>` and the matching block.",
        "todo functions DO NOT exist by default — build CRUD using `state::set`/`state::get`/`state::list`/`state::delete` keyed under `todo/*`.",
    ]);

    Ok(json!({
        "tips": pinned_tips,
        "connected_workers": connected,
        "engine_builtins_disabled": not_registered_builtins,
        "skill_index": skill_index,
        "discovery_flow": [
            "introspection::context::bootstrap",
            "introspection::functions::list { filter: '<keyword>' }",
            "introspection::functions::describe { id }",
            "agent_call { function, payload }",
        ],
    }))
}

fn is_anonymous_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if let Some(pos) = name.rfind(':') {
        let after = &name[pos + 1..];
        if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
            if pos == 0 || bytes[pos - 1] != b':' {
                return true;
            }
        }
    }
    false
}

pub async fn worker_status(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: name".into()))?;
    let raw = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;
    let workers = raw
        .get("workers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let row = workers
        .iter()
        .find(|w| w.get("name").and_then(|n| n.as_str()) == Some(name));
    let status = row
        .and_then(|w| w.get("status").and_then(|s| s.as_str()))
        .unwrap_or("not_registered")
        .to_string();
    let hint = builtin_hint(name);
    Ok(json!({
        "name": name,
        "status": status,
        "builtin": hint.is_some(),
        "activation_hint": hint,
    }))
}
