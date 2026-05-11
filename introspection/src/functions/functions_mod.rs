use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use super::{is_excluded, DEFAULT_EXCLUDED_NAMESPACES};

pub async fn list(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let worker_filter = payload.get("worker").and_then(|v| v.as_str());
    let id_filter = payload
        .get("filter")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());
    let include_noise = payload
        .get("include_noise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let extra_excludes: Vec<String> = payload
        .get("exclude_prefixes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let raw = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;

    let workers = raw
        .get("workers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut functions: Vec<Value> = Vec::new();
    let mut filtered_out = 0usize;
    for w in workers {
        if w.get("status").and_then(|s| s.as_str()) != Some("connected") {
            continue;
        }
        let worker_name = w.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if let Some(wf) = worker_filter {
            if worker_name != wf {
                continue;
            }
        }
        if let Some(fns) = w.get("functions").and_then(|f| f.as_array()) {
            for f in fns {
                let fn_id = match f {
                    Value::String(s) => s.clone(),
                    Value::Object(o) => o
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    _ => continue,
                };
                if fn_id.is_empty() {
                    continue;
                }
                if !include_noise && is_excluded(&fn_id, &extra_excludes) {
                    filtered_out += 1;
                    continue;
                }
                if let Some(q) = &id_filter {
                    if !fn_id.to_lowercase().contains(q) {
                        continue;
                    }
                }
                let description = match f {
                    Value::Object(o) => o.get("description").cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                functions.push(json!({
                    "id": fn_id,
                    "worker": worker_name,
                    "description": description,
                }));
            }
        }
    }

    Ok(json!({
        "count": functions.len(),
        "filtered_out": filtered_out,
        "exclude_prefixes_default": DEFAULT_EXCLUDED_NAMESPACES,
        "functions": functions,
    }))
}

pub async fn describe(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".into()))?;

    let raw = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;

    let workers = raw
        .get("workers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for w in workers {
        let worker_name = w.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if let Some(fns) = w.get("functions").and_then(|f| f.as_array()) {
            for f in fns {
                let fn_id = match f {
                    Value::String(s) => s.clone(),
                    Value::Object(o) => o
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    _ => continue,
                };
                if fn_id == id {
                    let mut out = match f {
                        Value::Object(o) => Value::Object(o.clone()),
                        _ => json!({"id": fn_id.clone()}),
                    };
                    if let Value::Object(ref mut map) = out {
                        map.insert("worker".into(), json!(worker_name));
                    }
                    return Ok(out);
                }
            }
        }
    }

    Err(IIIError::Handler(format!("function not found: {id}")))
}
