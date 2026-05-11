use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use super::{builtin_hint, is_excluded, ENGINE_BUILTINS};

pub async fn list(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let include = payload
        .get("include")
        .and_then(|v| v.as_str())
        .unwrap_or("slim");
    let filter = payload.get("filter").and_then(|v| v.as_str());
    let include_disconnected = payload
        .get("include_disconnected")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let raw = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;

    let mut workers = raw
        .get("workers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if let Some(q) = filter {
        let q = q.to_lowercase();
        workers.retain(|w| {
            w.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.to_lowercase().contains(&q))
                .unwrap_or(false)
        });
    }

    if !include_disconnected {
        workers.retain(|w| w.get("status").and_then(|s| s.as_str()) == Some("connected"));
    }

    if include == "slim" {
        workers = workers
            .into_iter()
            .map(|w| {
                let name_str = w
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = w.get("status").cloned().unwrap_or(Value::Null);
                let function_count = w
                    .get("function_count")
                    .cloned()
                    .or_else(|| {
                        w.get("functions")
                            .and_then(|f| f.as_array())
                            .map(|a| json!(a.len()))
                    })
                    .unwrap_or(json!(0));
                let description = w.get("description").cloned().unwrap_or(Value::Null);
                let mut entry = json!({
                    "name": name_str,
                    "status": status,
                    "function_count": function_count,
                    "description": description,
                });
                if let Some(hint) = builtin_hint(&name_str) {
                    if let Value::Object(map) = &mut entry {
                        map.insert("builtin".into(), json!(true));
                        map.insert("activation_hint".into(), json!(hint));
                    }
                }
                entry
            })
            .collect();
    }

    Ok(json!({
        "include": include,
        "count": workers.len(),
        "engine_builtins_known": ENGINE_BUILTINS.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "workers": workers,
    }))
}

pub async fn describe(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: name".into()))?;

    let raw = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;

    let worker = raw
        .get("workers")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|w| w.get("name").and_then(|n| n.as_str()) == Some(name))
        })
        .cloned();

    let mut out = match worker {
        Some(w) => w,
        None => {
            // Not on the bus. If it's a known engine builtin, return the hint.
            if let Some(hint) = builtin_hint(name) {
                return Ok(json!({
                    "name": name,
                    "status": "not_registered",
                    "builtin": true,
                    "activation_hint": hint,
                }));
            }
            return Err(IIIError::Handler(format!("worker not found: {name}")));
        }
    };

    // Slim the embedded function entries: drop excluded prefixes, keep
    // only id + description. Cuts a typical describe payload from ~30KB
    // to ~2KB without losing what the agent needs to plan.
    if let Some(fns) = out.get("functions").and_then(|v| v.as_array()).cloned() {
        let slim: Vec<Value> = fns
            .into_iter()
            .filter_map(|f| {
                let id = f
                    .get("id")
                    .or_else(|| if f.is_string() { Some(&f) } else { None })
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() || is_excluded(&id, &[]) {
                    return None;
                }
                Some(json!({
                    "id": id,
                    "description": f.get("description").cloned().unwrap_or(Value::Null),
                }))
            })
            .collect();
        if let Value::Object(map) = &mut out {
            map.insert("functions".into(), Value::Array(slim));
        }
    }

    if let Some(hint) = builtin_hint(name) {
        if let Value::Object(map) = &mut out {
            map.insert("builtin".into(), json!(true));
            map.insert("activation_hint".into(), json!(hint));
        }
    }

    // Resolve skills associated with this worker (best-effort).
    if let Some(skills) = list_skills_for(&iii, name).await {
        if let Value::Object(map) = &mut out {
            map.insert("skills".into(), Value::Array(skills));
        }
    }

    Ok(out)
}

async fn list_skills_for(iii: &Arc<III>, worker: &str) -> Option<Vec<Value>> {
    let resp = super::call(iii, "skills::list", json!({})).await.ok()?;
    let arr = resp.get("skills").and_then(|v| v.as_array())?;
    let prefix = format!("{worker}/");
    let mut out: Vec<Value> = Vec::new();
    for entry in arr {
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id == worker || id.starts_with(&prefix) {
            out.push(json!({
                "id": id,
                "uri": format!("iii://{id}"),
            }));
        }
    }
    Some(out)
}
