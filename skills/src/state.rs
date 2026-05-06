//! Thin StateKV wrappers around `iii.trigger()`. Kept in one place so
//! every function module uses the same scope + timeout conventions.

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

pub async fn state_set(
    iii: &III,
    scope: &str,
    key: &str,
    value: Value,
    timeout_ms: u64,
) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
}

pub async fn state_get(
    iii: &III,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
}

pub async fn state_delete(
    iii: &III,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
}

pub async fn state_list(iii: &III, scope: &str, timeout_ms: u64) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".to_string(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
}

/// Normalize the several shapes `state::list` can return (a flat array,
/// a `{items: [{value: ...}, ...]}` envelope, or `{value: [...]}`)
/// into a plain Vec<Value> of the stored values.
pub fn extract_state_entries(raw: Value) -> Vec<Value> {
    if let Value::Array(arr) = raw {
        return arr;
    }
    if let Some(items) = raw.get("items").and_then(|v| v.as_array()) {
        return items
            .iter()
            .filter_map(|item| item.get("value").cloned())
            .collect();
    }
    if let Some(arr) = raw.get("value").and_then(|v| v.as_array()) {
        return arr.clone();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_state_entries_handles_flat_array() {
        let raw = json!([{"id": "a"}, {"id": "b"}]);
        let entries = extract_state_entries(raw);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn extract_state_entries_handles_items_envelope() {
        let raw = json!({ "items": [{"value": {"id": "a"}}] });
        let entries = extract_state_entries(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "a");
    }

    #[test]
    fn extract_state_entries_handles_value_array() {
        let raw = json!({ "value": [{"id": "a"}] });
        let entries = extract_state_entries(raw);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn extract_state_entries_handles_unknown_shape() {
        let raw = json!({ "random": "not-a-list" });
        let entries = extract_state_entries(raw);
        assert_eq!(entries.len(), 0);
    }
}
