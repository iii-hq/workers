use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const STATE_TIMEOUT_MS: u64 = 5_000;

pub fn scope() -> String {
    "acp".to_string()
}

// Persisted keys are NOT scoped by conn_id. session_id is a globally
// unique uuid (sess_<32hex>) and must survive subprocess restarts so a
// reconnecting editor can resume an old thread via session/load. conn_id
// stays in-memory only as transient ownership metadata for routing
// agent::events to the right subprocess.
pub fn session_key(session_id: &str) -> String {
    format!("sessions:{}", session_id)
}

pub fn session_index_key() -> &'static str {
    "sessions:_index"
}

pub fn session_history_key(session_id: &str) -> String {
    format!("sessions:{}:history", session_id)
}

// Streaming wire = the iii ecosystem's `agent::events` stream. No
// per-connection topic exists. Brains (turn-orchestrator and any
// drop-in replacement) emit AgentEvent frames into that stream with
// group_id = session_id; iii-acp subscribes once and routes by group.
pub const AGENT_EVENTS_STREAM: &str = "agent::events";

pub fn cancel_topic(conn_id: &str, session_id: &str) -> String {
    format!("acp:{}:session:{}:cancel", conn_id, session_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub conn_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    pub created_at_ms: i64,
    pub last_activity_ms: i64,
    // Optional ACP mode set via session/set_mode. None until first set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    // Per-session config options set via session/set_config_option. Keys are
    // configId strings, values are arbitrary JSON.
    #[serde(default)]
    pub config_options: serde_json::Map<String, Value>,
}

pub async fn state_get(iii: &IIIClient, scope: &str, key: &str) -> Result<Option<Value>, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::get".to_string(),
            payload: json!({ "scope": scope, "key": key }),
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await;
    match result {
        Ok(val) => Ok(unwrap_value(val)),
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

pub async fn state_set(iii: &IIIClient, scope: &str, key: &str, value: Value) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn state_delete(iii: &IIIClient, scope: &str, key: &str) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn durable_publish(iii: &IIIClient, topic: &str, data: Value) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "iii::durable::publish".to_string(),
        payload: json!({ "topic": topic, "data": data }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

fn unwrap_value(v: Value) -> Option<Value> {
    if v.is_null() {
        return None;
    }
    if let Some(obj) = v.as_object() {
        if let Some(inner) = obj.get("value") {
            if inner.is_null() {
                return None;
            }
            return Some(inner.clone());
        }
        if obj.is_empty() {
            return None;
        }
    }
    Some(v)
}

// Read-modify-write. Engine `state::update` (this engine version) only ships
// set/merge/increment/decrement/remove ops — no array append, so atomic
// append at the engine level is not available. Concurrent writers for the
// same session race here. Caller (handler.rs) serializes via a per-session
// in-process mutex to close the window for the common case (multiple
// agent::events arriving in parallel from one stream subscriber).
pub async fn append_history(iii: &IIIClient, session_id: &str, entry: Value) -> Result<(), Error> {
    let scope = scope();
    let key = session_history_key(session_id);
    let mut hist = state_get(iii, &scope, &key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    hist.push(entry);
    state_set(iii, &scope, &key, Value::Array(hist)).await
}

pub async fn read_history(iii: &IIIClient, session_id: &str) -> Result<Vec<Value>, Error> {
    let scope = scope();
    let key = session_history_key(session_id);
    Ok(state_get(iii, &scope, &key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default())
}

pub async fn append_session_to_index(iii: &IIIClient, session_id: &str) -> Result<(), Error> {
    // Read-modify-write under in-process index mutex (caller-owned).
    let scope = scope();
    let key = session_index_key();
    let mut idx = state_get(iii, &scope, key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let new_entry = Value::String(session_id.to_string());
    if !idx.contains(&new_entry) {
        idx.push(new_entry);
        state_set(iii, &scope, key, Value::Array(idx)).await?;
    }
    Ok(())
}

pub async fn remove_session_from_index(iii: &IIIClient, session_id: &str) -> Result<(), Error> {
    // state::update has no array-element-by-value remove op, so this stays
    // a read-modify-write. Race window: a concurrent append from
    // session/new for a different id can be lost. Acceptable in practice
    // because session/close is single-user / single-action and the
    // sweeper-style use case isn't part of v0.
    let scope = scope();
    let key = session_index_key();
    let idx = state_get(iii, &scope, key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let entry = Value::String(session_id.to_string());
    let next: Vec<Value> = idx.into_iter().filter(|v| v != &entry).collect();
    state_set(iii, &scope, key, Value::Array(next)).await
}

pub async fn read_session_index(iii: &IIIClient) -> Result<Vec<String>, Error> {
    let scope = scope();
    let key = session_index_key();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in state_get(iii, &scope, key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
    {
        if let Some(s) = v.as_str() {
            // Dedupe on read — append_session_to_index uses an atomic
            // append, so the index can carry duplicates if the same id
            // ever lands twice.
            if seen.insert(s.to_string()) {
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_session_id_only() {
        assert_eq!(session_key("s1"), "sessions:s1");
        assert_eq!(session_index_key(), "sessions:_index");
        assert_eq!(session_history_key("s1"), "sessions:s1:history");
    }

    #[test]
    fn topics_namespace_globally() {
        assert_eq!(AGENT_EVENTS_STREAM, "agent::events");
        assert_eq!(cancel_topic("c1", "s1"), "acp:c1:session:s1:cancel");
    }

    #[test]
    fn unwrap_value_handles_envelope_and_bare() {
        assert_eq!(unwrap_value(json!(null)), None);
        assert_eq!(unwrap_value(json!({"value": null})), None);
        assert_eq!(unwrap_value(json!({"value": 42})), Some(json!(42)));
        assert_eq!(unwrap_value(json!({"a": 1})), Some(json!({"a": 1})));
        assert_eq!(unwrap_value(json!([1, 2, 3])), Some(json!([1, 2, 3])));
    }
}
