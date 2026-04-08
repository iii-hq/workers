use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::analysis::hash_path;
use crate::config::SensorConfig;
use crate::state;

pub async fn handle(iii: Arc<III>, _config: Arc<SensorConfig>, payload: Value) -> Result<Value, IIIError> {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: path".to_string()))?;

    let limit = payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let path_hash = hash_path(path);
    let scope = format!("sensor:history:{path_hash}");

    let scores_val = state::state_get(&iii, &scope, "scores").await;

    let scores: Vec<Value> = scores_val
        .ok()
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                serde_json::from_value(v).ok()
            }
        })
        .unwrap_or_default();

    let total = scores.len();
    let limited: Vec<Value> = scores.into_iter().rev().take(limit).collect();

    let trend = if limited.len() < 2 {
        "stable"
    } else {
        let first_score = limited
            .last()
            .and_then(|v| v.get("score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let last_score = limited
            .first()
            .and_then(|v| v.get("score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let delta = last_score - first_score;
        if delta > 2.0 {
            "improving"
        } else if delta < -2.0 {
            "degrading"
        } else {
            "stable"
        }
    };

    Ok(serde_json::json!({
        "scores": limited,
        "total_entries": total,
        "trend": trend,
    }))
}
