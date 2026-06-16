//! Thin `session::get` wrapper around `iii.trigger()`.

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

pub async fn get(iii: &III, session_id: &str, timeout_ms: Option<u64>) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "session::get".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms,
    })
    .await
}
