use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

const SCOPE: &str = "conductor";
const RUN_PREFIX: &str = "runs::";
const STATE_TIMEOUT_MS: u64 = 5_000;

use crate::types::RunState;

pub async fn write_run(iii: &III, run: &RunState) -> Result<(), IIIError> {
    let value = serde_json::to_value(run)
        .map_err(|e| IIIError::Handler(format!("serialize RunState: {e}")))?;
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({
            "scope": SCOPE,
            "key": format!("{RUN_PREFIX}{}", run.id),
            "value": value,
        }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn read_run(iii: &III, run_id: &str) -> Result<Option<RunState>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::get".to_string(),
            payload: json!({ "scope": SCOPE, "key": format!("{RUN_PREFIX}{run_id}") }),
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await;
    match result {
        Ok(val) => Ok(extract_run_value(&val)),
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("not found") || msg.contains("no such") {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

pub async fn list_runs(iii: &III) -> Result<Vec<RunState>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::list".to_string(),
            payload: json!({ "scope": SCOPE, "prefix": RUN_PREFIX }),
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await?;
    Ok(extract_run_items(&result))
}

fn extract_run_value(val: &Value) -> Option<RunState> {
    if val.is_null() {
        return None;
    }
    if let Some(obj) = val.as_object() {
        if let Some(inner) = obj.get("value") {
            if inner.is_null() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_value::<RunState>(inner.clone()) {
                return Some(parsed);
            }
        }
    }
    serde_json::from_value::<RunState>(val.clone()).ok()
}

fn extract_run_items(val: &Value) -> Vec<RunState> {
    let arr = if let Some(arr) = val.as_array() {
        arr.clone()
    } else if let Some(items) = val.get("items").and_then(Value::as_array) {
        items.clone()
    } else {
        return Vec::new();
    };
    arr.into_iter()
        .filter_map(|v| extract_run_value(&v))
        .collect()
}
