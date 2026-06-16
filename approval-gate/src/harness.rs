//! Thin `harness::function::resolve` wrapper around `iii.trigger()`.

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::Value;

pub async fn function_resolve(
    iii: &III,
    payload: Value,
    timeout_ms: Option<u64>,
) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "harness::function::resolve".into(),
        payload,
        action: None,
        timeout_ms,
    })
    .await
}
