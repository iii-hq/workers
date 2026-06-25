//! `llm-router` adapters: model-limit resolution via
//! `router::models::get` and the summariser via `router::chat`.
//!
//! Both are soft dependencies — when the router is absent the resolver
//! errors (callers degrade to the conservative fallback) and the
//! summariser reports `Unavailable` (compaction maps it to
//! `status: "overflow"`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::helpers::create_channel;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::configuration::ConfigCell;
use crate::ports::{ModelResolver, SummarizeError, SummarizeRequest, Summarizer};
use crate::types::Model;

const MODELS_GET_TIMEOUT_MS: u64 = 5_000;
/// Grace period for the stream reader to drain after `router::chat`
/// returns; the terminal `done`/`error` event lands before or with the
/// trigger response, so this only covers socket latency.
const READER_DRAIN_MS: u64 = 2_000;

fn is_unroutable(err: &Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("function_not_found") || msg.contains("not found") || msg.contains("no function")
}

/// `router::models::get` — `{ model: Model } | null`.
pub struct RouterModelResolver {
    iii: Arc<IIIClient>,
}

impl RouterModelResolver {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl ModelResolver for RouterModelResolver {
    async fn get_model(&self, provider: Option<&str>, id: &str) -> Result<Option<Model>, String> {
        let mut payload = json!({ "id": id });
        if let Some(p) = provider {
            payload["provider"] = json!(p);
        }
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::models::get".to_string(),
                payload,
                action: None,
                timeout_ms: Some(MODELS_GET_TIMEOUT_MS),
            })
            .await
            .map_err(|e| e.to_string())?;

        if resp.is_null() {
            return Ok(None);
        }
        let model = resp
            .get("model")
            .cloned()
            .ok_or_else(|| "router::models::get response has no `model` field".to_string())?;
        if model.is_null() {
            return Ok(None);
        }
        serde_json::from_value::<Model>(model)
            .map(Some)
            .map_err(|e| format!("router::models::get returned an unparseable model: {e}"))
    }
}

/// `router::chat` — one summariser turn streamed over a channel the
/// adapter creates; the summary text comes from the terminal `done`
/// event (accumulated `text_delta`s as fallback).
///
/// Holds the live [`ConfigCell`] rather than a captured timeout, so a
/// `summarizer_timeout_ms` change hot-applies on the next call with no
/// rebuild.
pub struct RouterSummarizer {
    iii: Arc<IIIClient>,
    config: ConfigCell,
}

impl RouterSummarizer {
    pub fn new(iii: Arc<IIIClient>, config: ConfigCell) -> Self {
        Self { iii, config }
    }
}

#[async_trait]
impl Summarizer for RouterSummarizer {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError> {
        let channel = create_channel(&self.iii, None)
            .await
            .map_err(|e| SummarizeError::Unavailable(format!("create_channel failed: {e}")))?;

        // Frames may arrive as WS text messages (dispatched to the
        // callback) or as binary chunks (returned by read_all); collect
        // both since the router's framing is provider-defined.
        let text_frames: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = text_frames.clone();
        let reader = channel.reader;
        reader
            .on_message(move |frame| {
                sink.lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push(frame);
            })
            .await;
        let read_task = tokio::spawn(async move { reader.read_all().await });

        let mut payload = json!({
            "writer_ref": channel.writer_ref,
            "model": req.model,
            "system_prompt": req.system_prompt,
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": req.user_prompt }],
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            }],
            "tools": [],
        });
        if let Some(p) = &req.provider {
            payload["provider"] = json!(p);
        }

        // Read the outer budget from the live snapshot so a
        // summarizer_timeout_ms change takes effect without a restart.
        let timeout_ms = self.config.read().await.summarizer_timeout_ms;
        let trigger_result = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::chat".to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await;

        // The terminal event precedes the trigger response; give the
        // socket a short drain window, then stop reading either way.
        let abort = read_task.abort_handle();
        let binary =
            match tokio::time::timeout(Duration::from_millis(READER_DRAIN_MS), read_task).await {
                Ok(Ok(Ok(bytes))) => bytes,
                _ => {
                    abort.abort();
                    Vec::new()
                }
            };

        let response = match trigger_result {
            Ok(v) => v,
            Err(e) if is_unroutable(&e) => {
                return Err(SummarizeError::Unavailable(e.to_string()));
            }
            Err(e) => return Err(SummarizeError::Failed(e.to_string())),
        };
        if response.get("ok").and_then(Value::as_bool) == Some(false) {
            let detail = response
                .get("error")
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown router error".to_string());
            return Err(SummarizeError::Failed(detail));
        }

        let mut frames: Vec<String> = text_frames
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        frames.extend(String::from_utf8_lossy(&binary).lines().map(str::to_string));

        let summary = extract_summary(&frames)?;
        if summary.is_empty() {
            return Err(SummarizeError::Empty);
        }
        Ok(summary)
    }
}

/// Fold a stream of `AssistantMessageEvent` JSON frames into the final
/// summary text: the `done` event's message wins; accumulated
/// `text_delta`s are the fallback; a terminal `error` event fails.
fn extract_summary(frames: &[String]) -> Result<String, SummarizeError> {
    let mut deltas = String::new();
    let mut done_text: Option<String> = None;

    for frame in frames {
        let Ok(event) = serde_json::from_str::<Value>(frame) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                if let Some(d) = event.get("delta").and_then(Value::as_str) {
                    deltas.push_str(d);
                }
            }
            Some("done") => {
                done_text = Some(text_of_message(event.get("message")));
            }
            Some("error") => {
                return Err(SummarizeError::Failed(format!(
                    "summariser stream error: {}",
                    text_of_message(event.get("error"))
                )));
            }
            Some("stop") if event.get("stop_reason").and_then(Value::as_str) == Some("error") => {
                let detail = event
                    .get("error_message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown provider error");
                return Err(SummarizeError::Failed(format!(
                    "summariser stream error: {detail}"
                )));
            }
            _ => {}
        }
    }

    Ok(done_text.filter(|t| !t.is_empty()).unwrap_or(deltas))
}

/// Concatenated text blocks of an `AssistantMessage`-shaped value.
fn text_of_message(message: Option<&Value>) -> String {
    let Some(blocks) = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(v: Value) -> String {
        v.to_string()
    }

    #[test]
    fn done_event_text_wins_over_deltas() {
        let frames = vec![
            frame(json!({ "type": "text_delta", "partial": {}, "delta": "par" })),
            frame(json!({ "type": "text_delta", "partial": {}, "delta": "tial" })),
            frame(json!({ "type": "done", "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "## Goal\n- finish" }]
            }})),
        ];
        assert_eq!(extract_summary(&frames).unwrap(), "## Goal\n- finish");
    }

    #[test]
    fn deltas_are_the_fallback_without_done() {
        let frames = vec![
            frame(json!({ "type": "text_delta", "delta": "a" })),
            frame(json!({ "type": "text_delta", "delta": "b" })),
            frame(json!({ "type": "ping" })),
        ];
        assert_eq!(extract_summary(&frames).unwrap(), "ab");
    }

    #[test]
    fn terminal_error_event_fails() {
        let frames = vec![frame(json!({ "type": "error", "error": {
            "content": [{ "type": "text", "text": "boom" }]
        }}))];
        let err = extract_summary(&frames).unwrap_err();
        assert!(matches!(err, SummarizeError::Failed(_)));
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn stop_with_error_reason_fails() {
        let frames = vec![frame(
            json!({ "type": "stop", "stop_reason": "error", "error_message": "ctx overflow" }),
        )];
        let err = extract_summary(&frames).unwrap_err();
        assert!(err.to_string().contains("ctx overflow"));
    }

    #[test]
    fn unparseable_frames_are_skipped() {
        let frames = vec![
            "not-json".to_string(),
            frame(json!({ "type": "text_delta", "delta": "ok" })),
        ];
        assert_eq!(extract_summary(&frames).unwrap(), "ok");
    }
}
