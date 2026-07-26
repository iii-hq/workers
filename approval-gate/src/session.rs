//! Thin `session::get` wrapper around `iii.trigger()`.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn get(iii: &IIIClient, session_id: &str) -> Result<Value, Error> {
    let request = TriggerRequest {
        function_id: "session::get".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: None,
    };
    match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await,
        None => iii.trigger(request).await,
    }
}
