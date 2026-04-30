use iii_sdk::{III, IIIError, TriggerRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const STATE_TIMEOUT_MS: u64 = 5_000;

pub fn scope() -> String {
    "acp".to_string()
}

pub fn session_key(conn_id: &str, session_id: &str) -> String {
    format!("{}:sessions:{}", conn_id, session_id)
}

pub fn session_index_key(conn_id: &str) -> String {
    format!("{}:sessions:_index", conn_id)
}

pub fn session_history_key(conn_id: &str, session_id: &str) -> String {
    format!("{}:sessions:{}:history", conn_id, session_id)
}

pub fn updates_topic(conn_id: &str, session_id: &str) -> String {
    format!("acp:{}:session:{}:updates", conn_id, session_id)
}

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
}

pub async fn state_get(iii: &III, scope: &str, key: &str) -> Result<Option<Value>, IIIError> {
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

pub async fn state_set(iii: &III, scope: &str, key: &str, value: Value) -> Result<(), IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn state_delete(iii: &III, scope: &str, key: &str) -> Result<(), IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn durable_publish(iii: &III, topic: &str, data: Value) -> Result<(), IIIError> {
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

pub async fn append_history(
    iii: &III,
    conn_id: &str,
    session_id: &str,
    entry: Value,
) -> Result<(), IIIError> {
    let scope = scope();
    let key = session_history_key(conn_id, session_id);
    let mut hist = state_get(iii, &scope, &key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    hist.push(entry);
    state_set(iii, &scope, &key, Value::Array(hist)).await
}

pub async fn read_history(
    iii: &III,
    conn_id: &str,
    session_id: &str,
) -> Result<Vec<Value>, IIIError> {
    let scope = scope();
    let key = session_history_key(conn_id, session_id);
    Ok(state_get(iii, &scope, &key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default())
}

pub async fn append_session_to_index(
    iii: &III,
    conn_id: &str,
    session_id: &str,
) -> Result<(), IIIError> {
    let scope = scope();
    let key = session_index_key(conn_id);
    let mut idx = state_get(iii, &scope, &key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let new_entry = Value::String(session_id.to_string());
    if !idx.contains(&new_entry) {
        idx.push(new_entry);
        state_set(iii, &scope, &key, Value::Array(idx)).await?;
    }
    Ok(())
}

pub async fn read_session_index(iii: &III, conn_id: &str) -> Result<Vec<String>, IIIError> {
    let scope = scope();
    let key = session_index_key(conn_id);
    Ok(state_get(iii, &scope, &key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect())
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
    fn keys_namespace_by_conn() {
        assert_eq!(session_key("c1", "s1"), "c1:sessions:s1");
        assert_eq!(session_index_key("c1"), "c1:sessions:_index");
        assert_eq!(session_history_key("c1", "s1"), "c1:sessions:s1:history");
    }

    #[test]
    fn topics_namespace_globally() {
        assert_eq!(updates_topic("c1", "s1"), "acp:c1:session:s1:updates");
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
