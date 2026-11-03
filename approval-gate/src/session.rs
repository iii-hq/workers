//! Thin `session::get` wrapper around `iii.call()`.

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use serde_json::{json, Value};

pub async fn get(iii: &IIIClient, session_id: &str) -> Result<Value, Error> {
    iii.call(CallRequest {
        function_id: "session::get".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: None,
    })
    .await
}
