//! `harness::send` / `stop` / `status` — turn execution.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ModelRef;

#[derive(Debug, Clone, Serialize)]
pub struct SendRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub message: String,
    pub model: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<SendOptions>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SendOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionsPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionsPolicy {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendResponse {
    pub session_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub accepted: bool,
}

pub async fn send(
    iii: &IIIClient,
    req: SendRequest,
    timeout_ms: u64,
) -> Result<SendResponse, Error> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "harness::send".into(),
            payload: serde_json::to_value(&req).unwrap_or(Value::Null),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    serde_json::from_value(value).map_err(|e| Error::Handler(format!("harness::send: {e}")))
}

pub async fn stop(iii: &IIIClient, session_id: &str, timeout_ms: u64) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "harness::stop".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

pub fn metadata(channel: &str, thread_ts: &str, model: &ModelRef) -> Value {
    json!({
        "surface": "slack",
        "slack_channel": channel,
        "slack_thread_ts": thread_ts,
        "model": model.id,
        "provider": model.provider,
    })
}
