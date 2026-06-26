//! Durable loop bookkeeping in iii state (harness.md § State).
//!
//! Two scopes: `harness_turn/<session_id>` holds the [`TurnRecord`] (loop
//! progress, per-send options, per-call checkpoints), and
//! `harness_idem/<idempotency_key>` holds the webhook-dedupe row. `state::get`
//! returns the stored value directly (null when absent); `state::delete`
//! returns the prior value.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::error::HarnessError;
use crate::types::turn::{IdemRecord, TurnRecord};

pub const TURN_SCOPE: &str = "harness_turn";
pub const IDEM_SCOPE: &str = "harness_idem";

async fn state_get(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Result<Value, HarnessError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
    .map_err(|e| HarnessError::State(format!("state::get {scope}/{key}: {e}")))
}

async fn state_set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: Value,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
    .map(|_| ())
    .map_err(|e| HarnessError::State(format!("state::set {scope}/{key}: {e}")))
}

async fn state_delete(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
    .map(|_| ())
    .map_err(|e| HarnessError::State(format!("state::delete {scope}/{key}: {e}")))
}

/// Read the turn record for a session (`None` when absent or null).
pub async fn get_turn(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Option<TurnRecord>, HarnessError> {
    let v = state_get(iii, TURN_SCOPE, session_id, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| HarnessError::State(format!("turn record parse: {e}")))
}

/// Persist the turn record (whole-record write; the loop holds the only
/// writer per session via the per-session lock).
pub async fn put_turn(
    iii: &IIIClient,
    record: &TurnRecord,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(record)
        .map_err(|e| HarnessError::State(format!("turn record serialize: {e}")))?;
    state_set(iii, TURN_SCOPE, &record.session_id, value, timeout_ms).await
}

pub async fn delete_turn(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    state_delete(iii, TURN_SCOPE, session_id, timeout_ms).await
}

/// List every turn record (the pending-call sweep scans these). `state::list`
/// returns a values array (or an object map); both shapes are tolerated.
pub async fn list_turns(iii: &IIIClient, timeout_ms: u64) -> Result<Vec<TurnRecord>, HarnessError> {
    let v = iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": TURN_SCOPE }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .map_err(|e| HarnessError::State(format!("state::list {TURN_SCOPE}: {e}")))?;
    Ok(parse_record_list(&v))
}

/// Tolerate the two `state::list` shapes seen across engines: a bare array of
/// values, or `{ "values": [...] }` / `{ "items": [...] }` / a key→value map.
fn parse_record_list(v: &Value) -> Vec<TurnRecord> {
    let candidates: Vec<&Value> = match v {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("values").or_else(|| map.get("items")) {
                items.iter().collect()
            } else {
                map.values().collect()
            }
        }
        _ => return Vec::new(),
    };
    candidates
        .into_iter()
        .filter_map(|c| serde_json::from_value::<TurnRecord>(c.clone()).ok())
        .collect()
}

pub async fn get_idem(
    iii: &IIIClient,
    key: &str,
    timeout_ms: u64,
) -> Result<Option<IdemRecord>, HarnessError> {
    let v = state_get(iii, IDEM_SCOPE, key, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| HarnessError::State(format!("idem record parse: {e}")))
}

pub async fn put_idem(
    iii: &IIIClient,
    key: &str,
    record: &IdemRecord,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(record)
        .map_err(|e| HarnessError::State(format!("idem record serialize: {e}")))?;
    state_set(iii, IDEM_SCOPE, key, value, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_record_list_handles_array_and_object_shapes() {
        let rec = json!({
            "turn_id": "t_1", "session_id": "s_1", "status": "running",
            "step": 0, "turn_count": 0, "depth": 0,
            "options": { "model": "m", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        });
        let as_array = json!([rec]);
        assert_eq!(parse_record_list(&as_array).len(), 1);
        let as_values = json!({ "values": [rec] });
        assert_eq!(parse_record_list(&as_values).len(), 1);
        let as_map = json!({ "s_1": rec });
        assert_eq!(parse_record_list(&as_map).len(), 1);
        assert_eq!(parse_record_list(&json!(null)).len(), 0);
    }
}
