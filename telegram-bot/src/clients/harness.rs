use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ModelRef;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HarnessSendRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub message: String,
    pub model: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSeed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<SendOptions>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SessionSeed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HarnessMode {
    Plan,
    Ask,
    Agent,
}

/// Mirrors the harness `SystemPromptStrategy`: `override` replaces the built-in
/// prompt, `enrich` appends to it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptStrategy {
    Override,
    Enrich,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SendOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_strategy: Option<SystemPromptStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<HarnessMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionsPolicy>,
    /// Tracing passthrough for harness (session_id / message_id propagate as baggage).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FunctionsPolicy {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HarnessSendResponse {
    pub session_id: String,
    pub turn_id: String,
    pub accepted: bool,
    #[serde(default)]
    pub merged: Option<bool>,
    #[serde(default)]
    pub deduplicated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HarnessStopRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HarnessStopResponse {
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HarnessStatusRequest {
    pub session_id: String,
}

pub async fn send(
    iii: &IIIClient,
    req: HarnessSendRequest,
    timeout_ms: u64,
) -> Result<HarnessSendResponse, Error> {
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

pub async fn stop(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<HarnessStopResponse, Error> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "harness::stop".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    serde_json::from_value(value).map_err(|e| Error::Handler(format!("harness::stop: {e}")))
}

pub async fn status_active(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<bool, Error> {
    let v = iii
        .trigger(TriggerRequest {
            function_id: "harness::status".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    Ok(!v.is_null())
}

pub fn metadata_for_chat(chat_id: i64, model: &ModelRef) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("telegram_chat_id".into(), json!(chat_id));
    m.insert("channel".into(), json!("telegram"));
    m.insert("model".into(), json!(model.id));
    m.insert("provider".into(), json!(model.provider));
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry;

    #[test]
    fn send_options_serializes_metadata() {
        let opts = SendOptions {
            system_prompt: None,
            system_prompt_strategy: None,
            mode: None,
            thinking_level: None,
            functions: None,
            metadata: Some(telemetry::tracing_metadata("s1", "tg-42")),
        };
        let v = serde_json::to_value(&opts).unwrap();
        assert_eq!(v["metadata"]["session_id"], "s1");
        assert_eq!(v["metadata"]["message_id"], "tg-42");
        assert_eq!(v["metadata"]["surface"], "telegram");
    }
}
