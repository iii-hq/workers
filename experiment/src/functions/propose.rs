use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::state;

pub async fn handle(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let experiment_id = payload
        .get("experiment_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: experiment_id".to_string()))?
        .to_string();

    let definition = state::state_get(&iii, "experiment:definitions", &experiment_id)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to load experiment definition: {e}")))?;

    if definition.is_null() {
        return Err(IIIError::Handler(format!(
            "experiment '{}' not found",
            experiment_id
        )));
    }

    let best_payload = state::state_get(&iii, "experiment:best", &experiment_id)
        .await
        .unwrap_or_else(|_| {
            definition
                .get("target_payload")
                .cloned()
                .unwrap_or(json!({}))
        });

    let base = if best_payload.is_null() {
        definition
            .get("target_payload")
            .cloned()
            .unwrap_or(json!({}))
    } else {
        best_payload
    };

    let modified_payload = vary_payload(&base);

    let proposal_id = Uuid::new_v4().to_string();
    let hypothesis = describe_changes(&base, &modified_payload);

    let proposal = json!({
        "proposal_id": proposal_id,
        "experiment_id": experiment_id,
        "hypothesis": hypothesis,
        "modified_payload": modified_payload,
        "base_payload": base,
    });

    let proposal_key = format!("{}:{}", experiment_id, proposal_id);
    state::state_set(
        &iii,
        "experiment:proposals",
        &proposal_key,
        proposal.clone(),
    )
    .await
    .map_err(|e| IIIError::Handler(format!("failed to save proposal: {e}")))?;

    Ok(json!({
        "proposal_id": proposal_id,
        "hypothesis": hypothesis,
        "modified_payload": modified_payload,
    }))
}

fn vary_payload(base: &Value) -> Value {
    match base {
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (key, val) in map {
                new_map.insert(key.clone(), vary_value(val));
            }
            Value::Object(new_map)
        }
        other => other.clone(),
    }
}

fn vary_value(val: &Value) -> Value {
    match val {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                let variation = pseudo_random_variation();
                let new_val = f * (1.0 + variation);
                json!(new_val)
            } else {
                val.clone()
            }
        }
        Value::Bool(b) => {
            let flip = pseudo_random_variation().abs() > 0.3;
            if flip {
                json!(!b)
            } else {
                json!(*b)
            }
        }
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (key, v) in map {
                new_map.insert(key.clone(), vary_value(v));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => {
            let new_arr: Vec<Value> = arr.iter().map(vary_value).collect();
            Value::Array(new_arr)
        }
        other => other.clone(),
    }
}

fn pseudo_random_variation() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    
    ((nanos % 1000) as f64 / 1000.0) - 0.5
}

fn describe_changes(base: &Value, modified: &Value) -> String {
    let mut changes = Vec::new();

    if let (Some(base_obj), Some(mod_obj)) = (base.as_object(), modified.as_object()) {
        for (key, base_val) in base_obj {
            if let Some(mod_val) = mod_obj.get(key) {
                if base_val != mod_val {
                    changes.push(format!("{}: {} -> {}", key, base_val, mod_val));
                }
            }
        }
    }

    if changes.is_empty() {
        "no parameter changes".to_string()
    } else {
        changes.join(", ")
    }
}
