//! Thin `harness::function::resolve` wrapper around `iii.trigger()`.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::Value;

pub async fn function_resolve(iii: &IIIClient, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "harness::function::resolve".into(),
        payload,
        action: None,
        timeout_ms: None,
    })
    .await
}
