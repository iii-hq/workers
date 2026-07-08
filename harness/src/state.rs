//! Durable loop bookkeeping in iii state (harness.md § State).
//!
//! Three scopes: `harness_turn/<session_id>` holds the [`TurnRecord`] (loop
//! progress, per-send options, per-call checkpoints),
//! `harness_idem/<idempotency_key>` holds the webhook-dedupe row, and
//! `harness_queue/<session_id>:<id>` holds one [`QueuedMessage`] per message
//! that arrived while a step was streaming (drained by the loop). `state::get`
//! returns the stored value directly (null when absent); `state::delete`
//! returns the prior value.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::HarnessError;
use crate::types::message::AgentMessage;
use crate::types::turn::{IdemRecord, TurnRecord};

pub const TURN_SCOPE: &str = "harness_turn";
pub const IDEM_SCOPE: &str = "harness_idem";
pub const QUEUE_SCOPE: &str = "harness_queue";

pub(crate) async fn state_get(
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

pub(crate) async fn state_set(
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

pub(crate) async fn state_delete(
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
    Ok(parse_list(&state_list(iii, TURN_SCOPE, timeout_ms).await?))
}

async fn state_list(iii: &IIIClient, scope: &str, timeout_ms: u64) -> Result<Value, HarnessError> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".into(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
    .map_err(|e| HarnessError::State(format!("state::list {scope}: {e}")))
}

/// Tolerate the two `state::list` shapes seen across engines: a bare array of
/// values, or `{ "values": [...] }` / `{ "items": [...] }` / a key→value map.
fn parse_list<T: serde::de::DeserializeOwned>(v: &Value) -> Vec<T> {
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
        .filter_map(|c| serde_json::from_value::<T>(c.clone()).ok())
        .collect()
}

/// One message parked while a step was streaming, waiting for the loop's
/// drain to append it to the transcript (harness.md § Concurrency & steering).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueuedMessage {
    pub id: String,
    pub session_id: String,
    pub message: AgentMessage,
    /// Deterministic transcript entry id the drain appends under, so a
    /// redelivered drain is a no-op.
    pub entry_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Value>,
    pub queued_at: i64,
}

/// Enqueue a message under a fresh unique key — a blind write, safe without
/// the session lock.
pub async fn enqueue_message(
    iii: &IIIClient,
    row: &QueuedMessage,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let key = queue_key(&row.session_id, &row.id);
    let value = serde_json::to_value(row)
        .map_err(|e| HarnessError::State(format!("queued message serialize: {e}")))?;
    state_set(iii, QUEUE_SCOPE, &key, value, timeout_ms).await
}

/// The session's queued messages in arrival order (`queued_at`, then `id`).
// ponytail: state::list scans the whole scope; per-session prefix listing if
// queue volume matters.
pub async fn list_queued(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Vec<QueuedMessage>, HarnessError> {
    let mut rows: Vec<QueuedMessage> = parse_list(&state_list(iii, QUEUE_SCOPE, timeout_ms).await?)
        .into_iter()
        .filter(|r: &QueuedMessage| r.session_id == session_id)
        .collect();
    sort_queued(&mut rows);
    Ok(rows)
}

fn sort_queued(rows: &mut [QueuedMessage]) {
    rows.sort_by(|a, b| (a.queued_at, &a.id).cmp(&(b.queued_at, &b.id)));
}

pub async fn delete_queued(
    iii: &IIIClient,
    session_id: &str,
    id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    state_delete(iii, QUEUE_SCOPE, &queue_key(session_id, id), timeout_ms).await
}

fn queue_key(session_id: &str, id: &str) -> String {
    format!("{session_id}:{id}")
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
    fn parse_list_handles_array_and_object_shapes() {
        let rec = json!({
            "turn_id": "t_1", "session_id": "s_1", "status": "running",
            "step": 0, "turn_count": 0, "depth": 0,
            "options": { "model": "m", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        });
        let as_array = json!([rec]);
        assert_eq!(parse_list::<TurnRecord>(&as_array).len(), 1);
        let as_values = json!({ "values": [rec] });
        assert_eq!(parse_list::<TurnRecord>(&as_values).len(), 1);
        let as_map = json!({ "s_1": rec });
        assert_eq!(parse_list::<TurnRecord>(&as_map).len(), 1);
        assert_eq!(parse_list::<TurnRecord>(&json!(null)).len(), 0);
    }

    #[test]
    fn queued_messages_sort_by_arrival_then_id() {
        fn row(id: &str, at: i64) -> QueuedMessage {
            QueuedMessage {
                id: id.into(),
                session_id: "s_1".into(),
                message: AgentMessage::user_text("hi"),
                entry_id: format!("e_{id}"),
                origin: None,
                queued_at: at,
            }
        }
        let mut rows = vec![row("q_b", 2), row("q_c", 1), row("q_a", 2)];
        sort_queued(&mut rows);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["q_c", "q_a", "q_b"]);
    }
}
