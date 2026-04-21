use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

// Per-request timeout for all state helpers on the hot path. Routing has a
// sub-2s SLO so a stalled backend must error fast, not hang.
const STATE_TIMEOUT_MS: u64 = 1_500;

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
        Ok(val) => extract_value(&val, scope, key),
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

pub async fn state_list(iii: &III, scope: &str, prefix: &str) -> Result<Vec<Value>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::list".to_string(),
            payload: json!({ "scope": scope, "prefix": prefix }),
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await?;
    extract_items(&result, scope, prefix)
}

// state::get envelope. iii-sdk may return { "value": ... } or the value
// directly depending on engine version. Treat null as absent; reject malformed
// responses instead of silently returning None.
fn extract_value(val: &Value, scope: &str, key: &str) -> Result<Option<Value>, IIIError> {
    if val.is_null() {
        return Ok(None);
    }
    match val {
        Value::Object(m) => match m.get("value") {
            Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(v.clone())),
            None => {
                // An object without a "value" key is the raw stored value.
                // Preserve prior behavior, don't error.
                if m.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(Value::Object(m.clone())))
                }
            }
        },
        other => Ok(Some(other.clone())),
        // Suppress unreachable arm — above already covers all variants.
        #[allow(unreachable_patterns)]
        _ => Err(IIIError::Handler(format!(
            "malformed state::get response for scope={} key={}",
            scope, key
        ))),
    }
}

// state::list envelope. Handle three shapes seen across engine versions:
//   { "items": [...] }  (0.11.0)
//   [ ... ]              (0.11.2 bare array)
//   null                 (empty)
// Anything else is a hard error, not a silent empty.
fn extract_items(val: &Value, scope: &str, prefix: &str) -> Result<Vec<Value>, IIIError> {
    if val.is_null() {
        return Ok(Vec::new());
    }
    if let Some(arr) = val.as_array() {
        return Ok(arr.clone());
    }
    if let Some(items) = val.get("items") {
        if let Some(arr) = items.as_array() {
            return Ok(arr.clone());
        }
        return Err(IIIError::Handler(format!(
            "malformed state::list response: 'items' not an array (scope={} prefix={})",
            scope, prefix
        )));
    }
    Err(IIIError::Handler(format!(
        "malformed state::list response: missing 'items' and not an array (scope={} prefix={})",
        scope, prefix
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_value_unwraps_envelope() {
        let v = json!({"value": {"a": 1}});
        assert_eq!(
            extract_value(&v, "s", "k").unwrap(),
            Some(json!({"a": 1}))
        );
    }

    #[test]
    fn extract_value_treats_null_value_as_absent() {
        let v = json!({"value": null});
        assert_eq!(extract_value(&v, "s", "k").unwrap(), None);
    }

    #[test]
    fn extract_value_accepts_bare_object() {
        let v = json!({"a": 1});
        assert_eq!(
            extract_value(&v, "s", "k").unwrap(),
            Some(json!({"a": 1}))
        );
    }

    #[test]
    fn extract_items_accepts_wrapped_array() {
        let v = json!({"items": [{"a": 1}]});
        assert_eq!(extract_items(&v, "s", "p").unwrap(), vec![json!({"a": 1})]);
    }

    #[test]
    fn extract_items_accepts_bare_array() {
        let v = json!([{"a": 1}]);
        assert_eq!(extract_items(&v, "s", "p").unwrap(), vec![json!({"a": 1})]);
    }

    #[test]
    fn extract_items_rejects_bad_shape() {
        let v = json!({"items": "oops"});
        assert!(extract_items(&v, "s", "p").is_err());
    }

    #[test]
    fn extract_items_rejects_missing() {
        let v = json!({"other": 1});
        assert!(extract_items(&v, "s", "p").is_err());
    }
}
