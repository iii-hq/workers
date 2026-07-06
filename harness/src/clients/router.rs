//! `llm-router` client: stream a completion (`router::chat`), abort one
//! (`router::abort`), and read model capabilities (`router::models::get` /
//! `router::models::supports`).
//!
//! `chat` opens a harness-owned channel, hands the router its write end, and
//! reads `AssistantMessageEvent` frames off the read end while the call is
//! held open. Intermediate partials are forwarded to a [`StreamSink`]
//! (coalesced by `coalesce_ms`); the final assembled message comes from the
//! terminal `done` / `error` frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use iii_helpers::observability::opentelemetry::trace::FutureExt as _;
use iii_sdk::helpers::create_channel;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::sync::Notify;

use crate::error::HarnessError;
use crate::types::event::{AssistantMessageEvent, StopReason};
use crate::types::message::{empty_assistant, AssistantMessage};
use crate::types::model::{AgentFunction, Model, ThinkingLevel};

/// Receives coalesced partial assistant messages as a stream progresses.
#[async_trait]
pub trait StreamSink: Send + Sync {
    async fn on_update(&self, message: &AssistantMessage);
}

/// Inputs for one `router::chat` turn.
pub struct ChatParams {
    pub request_id: String,
    pub model: String,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub messages: Vec<Value>,
    pub tools: Vec<AgentFunction>,
    pub response_format: Option<Value>,
    pub thinking_level: Option<ThinkingLevel>,
}

/// The assembled result of one stream.
pub struct ChatOutcome {
    pub message: AssistantMessage,
    pub ok: bool,
    pub stop_reason: Option<StopReason>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct RouterClient {
    iii: Arc<IIIClient>,
    timeout_ms: u64,
    coalesce_ms: u64,
}

impl RouterClient {
    pub fn new(iii: Arc<IIIClient>, timeout_ms: u64, coalesce_ms: u64) -> Self {
        Self {
            iii,
            timeout_ms,
            coalesce_ms,
        }
    }

    /// Stream one assistant turn, forwarding coalesced partials to `sink`.
    pub async fn chat(
        &self,
        params: ChatParams,
        sink: &dyn StreamSink,
    ) -> Result<ChatOutcome, HarnessError> {
        let channel = create_channel(&self.iii, None)
            .await
            .map_err(|e| HarnessError::Dependency(format!("create_channel: {e}")))?;

        // Bridge the sync on_message callback into an async mpsc, and drive
        // next_binary in a pump task (the reader only dispatches text frames
        // while it is polled). The on_message closure holds the only live
        // sender; reader EOF drops it and ends our consume loop.
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        {
            let cb_tx = tx.clone();
            channel
                .reader
                .on_message(move |frame| {
                    let _ = cb_tx.send(frame);
                })
                .await;
        }
        drop(tx);
        let reader = channel.reader;
        let cancel = Arc::new(Notify::new());
        let cancel_pump = cancel.clone();
        let pump = tokio::spawn(async move {
            loop {
                tokio::select! {
                    r = reader.next_binary() => {
                        if !matches!(r, Ok(Some(_))) {
                            break;
                        }
                    }
                    _ = cancel_pump.notified() => {
                        let _ = reader.close().await;
                        break;
                    }
                }
            }
        });

        let mut payload = json!({
            "writer_ref": channel.writer_ref,
            "request_id": params.request_id,
            "model": params.model,
            "messages": params.messages,
            "tools": params.tools,
        });
        if let Some(p) = &params.provider {
            payload["provider"] = json!(p);
        }
        if let Some(sp) = &params.system_prompt {
            payload["system_prompt"] = json!(sp);
        }
        if let Some(rf) = &params.response_format {
            payload["response_format"] = rf.clone();
        }
        if let Some(tl) = &params.thinking_level {
            payload["thinking_level"] = serde_json::to_value(tl).unwrap_or(Value::Null);
        }

        // Run the held-open trigger concurrently with frame consumption.
        //
        // Carry the caller's OTel context into the spawned task so the SDK
        // injects the turn's traceparent and `router::chat` nests under
        // `harness::turn` instead of rooting a detached trace. `with_context`
        // (not `cx.attach()`) because the `ContextGuard` is `!Send` and can't
        // be held across the `.await` inside `tokio::spawn` — the same idiom
        // the SDK uses to attach a handler's context.
        let iii = self.iii.clone();
        let timeout_ms = self.timeout_ms;
        let parent_cx = iii_helpers::observability::opentelemetry::Context::current();
        let trigger = tokio::spawn(
            async move {
                iii.trigger(TriggerRequest {
                    function_id: "router::chat".into(),
                    payload,
                    action: None,
                    timeout_ms: Some(timeout_ms),
                })
                .await
            }
            .with_context(parent_cx),
        );

        let coalesce = Duration::from_millis(self.coalesce_ms);
        let mut last_emit = Instant::now()
            .checked_sub(coalesce)
            .unwrap_or_else(Instant::now);
        let mut final_message: Option<AssistantMessage> = None;
        let mut terminal_error: Option<String> = None;

        while let Some(frame) = rx.recv().await {
            let Ok(event) = serde_json::from_str::<AssistantMessageEvent>(&frame) else {
                continue;
            };
            match event {
                AssistantMessageEvent::Done { message } => {
                    sink.on_update(&message).await;
                    final_message = Some(message);
                }
                AssistantMessageEvent::Error { error } => {
                    terminal_error = error
                        .error_message
                        .clone()
                        .or_else(|| Some("stream error".to_string()));
                    final_message = Some(error);
                }
                AssistantMessageEvent::Stop {
                    error_message: Some(msg),
                    ..
                } => {
                    terminal_error.get_or_insert(msg);
                }
                other => {
                    if let Some(partial) = partial_of(&other) {
                        if last_emit.elapsed() >= coalesce {
                            sink.on_update(partial).await;
                            last_emit = Instant::now();
                        }
                    }
                }
            }
        }

        // Stream drained; stop the pump and collect the trigger ack.
        cancel.notify_waiters();
        let _ = pump.await;
        let response = trigger
            .await
            .map_err(|e| HarnessError::Internal(format!("router::chat task: {e}")))?;

        let (ok, response_error) = match &response {
            Ok(v) => {
                let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(true);
                let err = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                (ok, err)
            }
            Err(e) => (false, Some(e.to_string())),
        };

        let message = final_message.unwrap_or_else(|| {
            let mut m = empty_assistant(params.provider.as_deref().unwrap_or(""), &params.model);
            m.stop_reason = StopReason::Error;
            m.error_message = response_error
                .clone()
                .or_else(|| terminal_error.clone())
                .or_else(|| Some("router produced no terminal frame".to_string()));
            m.error_kind = Some(crate::types::event::ErrorKind::Transient);
            m
        });

        let stop_reason = Some(message.stop_reason);
        let error = terminal_error
            .or(response_error)
            .filter(|_| !ok || message.stop_reason == StopReason::Error);
        Ok(ChatOutcome {
            ok: ok && error.is_none(),
            message,
            stop_reason,
            error,
        })
    }

    /// Abort an in-flight stream by `request_id` (best-effort).
    pub async fn abort(&self, request_id: &str) -> bool {
        let res = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::abort".into(),
                payload: json!({ "request_id": request_id }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        match res {
            Ok(v) => v.get("aborted").and_then(Value::as_bool).unwrap_or(false),
            Err(e) => {
                tracing::warn!(request_id, error = %e, "router::abort failed");
                false
            }
        }
    }

    /// Effective per-provider identity prompt from the router: operator
    /// override → provider-declared → `None`. `None` also when the router is
    /// absent/old or the provider's prompt is disabled — the caller falls back
    /// to the embedded default prompt.
    pub async fn system_prompt_get(&self, provider: Option<&str>) -> Option<String> {
        let mut payload = json!({});
        if let Some(p) = provider {
            payload["provider"] = json!(p);
        }
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::system_prompt::get".into(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .ok()?;
        resp.get("system_prompt")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    }

    /// Look up one model's capabilities (`None` when unregistered or router
    /// absent — the caller degrades).
    pub async fn models_get(&self, provider: Option<&str>, id: &str) -> Option<Model> {
        let mut payload = json!({ "id": id });
        if let Some(p) = provider {
            payload["provider"] = json!(p);
        }
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::models::get".into(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .ok()?;
        if resp.is_null() {
            return None;
        }
        let model = resp.get("model").cloned().unwrap_or(Value::Null);
        if model.is_null() {
            return None;
        }
        serde_json::from_value::<Model>(model).ok()
    }

    /// Whether `model` supports a capability (false when the router is absent
    /// or the model is unknown — the caller falls back).
    pub async fn models_supports(&self, provider: &str, id: &str, capability: &str) -> bool {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::models::supports".into(),
                payload: json!({ "provider": provider, "id": id, "capability": capability }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        match resp {
            Ok(v) => v.get("supported").and_then(Value::as_bool).unwrap_or(false),
            Err(_) => false,
        }
    }
}

/// A no-op sink used when a turn does not stream into a session entry.
pub struct NullSink;

#[async_trait]
impl StreamSink for NullSink {
    async fn on_update(&self, _message: &AssistantMessage) {}
}

/// A sink backed by a closure-free shared latest-partial holder (used in
/// tests).
pub struct CapturingSink {
    pub updates: Arc<Mutex<Vec<AssistantMessage>>>,
    pub closed: Arc<AtomicBool>,
}

impl Default for CapturingSink {
    fn default() -> Self {
        Self {
            updates: Arc::new(Mutex::new(Vec::new())),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl StreamSink for CapturingSink {
    async fn on_update(&self, message: &AssistantMessage) {
        if !self.closed.load(Ordering::SeqCst) {
            self.updates
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(message.clone());
        }
    }
}

fn partial_of(event: &AssistantMessageEvent) -> Option<&AssistantMessage> {
    match event {
        AssistantMessageEvent::Start { partial }
        | AssistantMessageEvent::TextStart { partial }
        | AssistantMessageEvent::TextDelta { partial, .. }
        | AssistantMessageEvent::TextEnd { partial }
        | AssistantMessageEvent::ThinkingStart { partial }
        | AssistantMessageEvent::ThinkingDelta { partial, .. }
        | AssistantMessageEvent::ThinkingEnd { partial }
        | AssistantMessageEvent::FunctioncallStart { partial }
        | AssistantMessageEvent::FunctioncallDelta { partial, .. }
        | AssistantMessageEvent::FunctioncallEnd { partial } => Some(partial),
        _ => None,
    }
}
