pub mod engine;

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::Value;

/// Function registration is async (the SDK sends RegisterFunction frames in
/// the background), so retry a trigger until the function is routable — up
/// to ~5s — instead of sleeping a fixed amount.
pub async fn trigger_until_ready(
    iii: &Arc<IIIClient>,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for _ in 0..25 {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(format!("'{function_id}' never became callable: {last_err}"))
}
