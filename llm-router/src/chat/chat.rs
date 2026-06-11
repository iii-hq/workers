//! `router::chat` orchestration: validate → decide → gates → budget → attempt
//! loop (relay + invisible pre-first-frame retries) → exactly one terminal.
//! Each attempt invokes the provider's `provider::<id>::stream` iii function
//! via `iii.trigger` with a fresh router-owned iii channel.
//!
//! Engine-backed coverage: tests/integration.rs (happy path, cancellation,
//! abort, retry).
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use crate::types::errors::{RouterCode, RouterError};
use crate::types::events::{AssistantMessageEvent, StopReason};
use crate::types::router::{ChatResponse, ErrorShape};
use iii_sdk::{IIIError, TriggerRequest, III};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::catalog::queries::model_supports;
use crate::catalog::store::CatalogStore;
use crate::channels::create_router_channel;
use crate::config::entry::read_entry_value;
use crate::registry::store::RegistryStore;
use crate::routing::{decide, DecideInput};
use crate::settings::{provider_slices, RouterSettings};
use crate::triggers;

use super::inflight::InflightMap;
use super::output_tokens::resolve_max_output_tokens;
use super::relay::{relay_frames, FrameSink, NoTerminalReason, RelayOpts, RelayResult};
use super::retry::backoff_ms;
use super::synthesize::{synthesize_aborted, synthesize_error};

/// ChatRequest minus writer_ref (the sink arrives separately — the function
/// handler wraps the looked-up writer, complete wraps its own).
#[derive(Debug, Deserialize)]
pub struct ChatCall {
    pub request_id: Option<String>,
    pub model: String,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub messages: Value, // forwarded verbatim; validated to be an array
    pub tools: Option<Value>,
    pub response_format: Option<Value>,
    pub thinking_level: Option<Value>,
    pub max_output_tokens: Option<u64>,
    pub provider_options: Option<Value>,
    #[allow(dead_code)]
    pub metadata: Option<Value>,
}

pub struct ChatPipeline {
    pub iii: III,
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub inflight: Arc<InflightMap>,
    pub settings: Arc<RwLock<RouterSettings>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use crate::types::errors::is_function_not_found;

fn is_router_coded(err: &IIIError) -> bool {
    matches!(err, IIIError::Remote { code, .. } if code.starts_with("router/"))
}

impl ChatPipeline {
    pub async fn run(
        &self,
        call: ChatCall,
        sink: Arc<dyn FrameSink>,
    ) -> Result<ChatResponse, IIIError> {
        // A pre-stream failure must still leave exactly one terminal frame on
        // the sink. Without it, `router::complete`'s drain blocks for its full
        // reader budget and `router::chat` consumers never see a terminal.
        // (Regression: pre_stream_routing_failure_emits_one_error_terminal_frame.)
        let fail_pre_stream = |provider: &str, code: RouterCode, message: String| -> IIIError {
            let frame = synthesize_error(None, &call.model, provider, &message, None, now_ms());
            let _ = sink.send(&serde_json::to_string(&frame).expect("serializable frame"));
            RouterError::new(code, message).into()
        };

        // ── validate (pre-stream typed throws) ──
        if call.model.is_empty() {
            return Err(fail_pre_stream(
                "",
                RouterCode::InvalidRequest,
                "model is required".into(),
            ));
        }
        if !call.messages.is_array() {
            return Err(fail_pre_stream(
                "",
                RouterCode::InvalidRequest,
                "messages must be an array".into(),
            ));
        }

        let settings = self.settings.read().unwrap().clone();
        let candidates = decide(&DecideInput {
            model: call.model.clone(),
            provider: call.provider.clone(),
            registered_providers: self.registry.ids().await,
            catalog: self.catalog.model_ids().await,
            heuristics: settings.routing_heuristics.clone(),
            default_provider: settings.default_provider.clone(),
        })
        .map_err(|e| fail_pre_stream("", e.code, e.message))?;
        let provider = candidates[0].clone(); // MVP consumes candidates[0]
        let record = match self.registry.get(&provider).await {
            Some(record) => record,
            None => {
                return Err(fail_pre_stream(
                    &provider,
                    RouterCode::UnknownProvider,
                    format!("unknown provider {provider}"),
                ))
            }
        };

        // Structured-output gate: known model without the flag throws; unknown
        // model fails open — the provider is the final arbiter.
        let model_meta = self.catalog.get(&provider, &call.model).await;
        if call.response_format.is_some() {
            if let Some(meta) = &model_meta {
                if !model_supports(meta, "structured_output") {
                    return Err(fail_pre_stream(
                        &provider,
                        RouterCode::StructuredOutputUnsupported,
                        format!("structured output unsupported for model {}", call.model),
                    ));
                }
            }
        }

        let entry = read_entry_value(&self.iii).await;
        let slice = provider_slices(&entry)
            .get(&provider)
            .cloned()
            .unwrap_or(Value::Null);
        let max_output_tokens = resolve_max_output_tokens(
            call.max_output_tokens,
            slice.get("max_tokens").and_then(Value::as_u64),
            model_meta.as_ref().map(|m| m.max_output_tokens),
            record
                .declaration
                .defaults
                .as_ref()
                .and_then(|d| d.max_tokens)
                .unwrap_or(8192),
            settings.output_token_max,
        );

        let request_id = call
            .request_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let entry_handle = self.inflight.insert(&request_id);
        let result = self
            .attempts(
                &call,
                &provider,
                model_meta.as_ref(),
                max_output_tokens,
                &settings,
                &entry_handle,
                sink,
            )
            .await;
        self.inflight.remove(&request_id);
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn attempts(
        &self,
        call: &ChatCall,
        provider: &str,
        model_meta: Option<&crate::types::model::Model>,
        max_output_tokens: u64,
        settings: &RouterSettings,
        inflight: &super::inflight::InflightEntry,
        sink: Arc<dyn FrameSink>,
    ) -> Result<ChatResponse, IIIError> {
        let pricing = model_meta.and_then(|m| m.pricing.clone());
        let mut last_partial = None;
        let mut last_usage = None;

        let send_frame = |frame: &AssistantMessageEvent| {
            let _ = sink.send(&serde_json::to_string(frame).expect("serializable frame"));
        };
        let respond_err = |stop: StopReason, code: &str, message: String, usage| ChatResponse {
            ok: false,
            provider: provider.to_string(),
            model: call.model.clone(),
            stop_reason: Some(stop),
            usage,
            error: Some(ErrorShape {
                code: code.to_string(),
                message,
            }),
        };

        let max_attempts = 1 + settings.retry_max;
        for attempt in 1..=max_attempts {
            if inflight.aborted.load(Ordering::SeqCst) {
                break;
            }
            let channel = create_router_channel(&self.iii).await?;
            let mut reader = channel.reader;
            inflight.set_closer(reader.closer());

            let stream_input = json!({
                "writer_ref": channel.writer_ref,
                "system_prompt": call.system_prompt,
                "model": call.model,
                "messages": call.messages,
                "tools": call.tools,
                "response_format": call.response_format,
                "thinking_level": call.thinking_level,
                "max_output_tokens": max_output_tokens,
                "provider_options": call.provider_options.as_ref().and_then(|o| o.get(provider)).cloned(),
                "model_meta": model_meta,
            });

            // The provider call runs concurrently with the relay; if it throws
            // pre-stream, closing the reader unblocks the loop immediately.
            let iii = self.iii.clone();
            let function_id = format!("provider::{provider}::stream");
            let closer = reader.closer();
            let timeout = settings.stream_timeout_ms;
            let call_task = tokio::spawn(async move {
                let out = iii
                    .trigger(TriggerRequest {
                        function_id,
                        payload: stream_input,
                        action: None,
                        timeout_ms: Some(timeout),
                    })
                    .await;
                if out.is_err() {
                    closer();
                }
                out
            });

            let relay = relay_frames(
                &mut reader,
                sink.as_ref(),
                &RelayOpts {
                    idle: std::time::Duration::from_millis(settings.idle_timeout_ms),
                    pricing: pricing.clone(),
                    aborted: inflight.aborted.clone(),
                },
            )
            .await;
            let call_outcome = call_task
                .await
                .unwrap_or(Err(IIIError::Handler("provider task panicked".into())));

            match relay {
                RelayResult::Done { terminal, .. } => {
                    let AssistantMessageEvent::Done { message } = terminal else {
                        unreachable!()
                    };
                    return Ok(ChatResponse {
                        ok: true,
                        provider: provider.to_string(),
                        model: call.model.clone(),
                        stop_reason: Some(message.stop_reason),
                        usage: message.usage,
                        error: None,
                    });
                }
                RelayResult::ErrorFrame {
                    terminal,
                    forwarded,
                    terminal_forwarded,
                } => {
                    let AssistantMessageEvent::Error { error } = &terminal else {
                        unreachable!()
                    };
                    let kind = error.error_kind;
                    let can_retry = !forwarded
                        && kind.map(|k| k.is_retryable()).unwrap_or(false)
                        && attempt < max_attempts
                        && !inflight.aborted.load(Ordering::SeqCst);
                    if can_retry {
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms(
                            attempt, 500, 8000, rand_unit,
                        )))
                        .await;
                        continue;
                    }
                    if !terminal_forwarded {
                        send_frame(&terminal);
                    }
                    let AssistantMessageEvent::Error { error } = terminal else {
                        unreachable!()
                    };
                    let code = kind
                        .and_then(|k| serde_json::to_value(k).ok())
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "transient".into());
                    return Ok(respond_err(
                        StopReason::Error,
                        &code,
                        error
                            .error_message
                            .unwrap_or_else(|| "provider error".into()),
                        error.usage,
                    ));
                }
                RelayResult::Aborted { partial, usage, .. } => {
                    last_partial = partial;
                    last_usage = usage;
                    break;
                }
                RelayResult::CallerGone { .. } => {
                    // Consumer abandoned the turn: reader-close propagation IS the cancel.
                    return Ok(ChatResponse {
                        ok: true,
                        provider: provider.to_string(),
                        model: call.model.clone(),
                        stop_reason: Some(StopReason::Aborted),
                        usage: None,
                        error: None,
                    });
                }
                RelayResult::NoTerminal {
                    reason,
                    partial,
                    usage,
                    forwarded,
                } => {
                    last_partial = partial;
                    last_usage = usage.clone();
                    if let Err(err) = call_outcome {
                        if !forwarded {
                            if is_router_coded(&err) {
                                return Err(err);
                            }
                            if is_function_not_found(&err) {
                                if self.registry.set_availability(provider, false).await {
                                    triggers::publish(
                                        &self.iii,
                                        triggers::PROVIDER_CHANGED,
                                        json!({ "provider": provider, "op": "unavailable" }),
                                    )
                                    .await;
                                }
                                return Err(RouterError::new(
                                    RouterCode::ProviderUnavailable,
                                    format!("provider {provider} unavailable"),
                                )
                                .into());
                            }
                        }
                    }
                    let is_idle = matches!(reason, NoTerminalReason::Idle);
                    let can_retry = !forwarded
                        && !is_idle
                        && attempt < max_attempts
                        && !inflight.aborted.load(Ordering::SeqCst);
                    if can_retry {
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms(
                            attempt, 500, 8000, rand_unit,
                        )))
                        .await;
                        continue;
                    }
                    let message = if is_idle {
                        format!(
                            "provider {provider} stream idle past {}ms",
                            settings.idle_timeout_ms
                        )
                    } else {
                        format!("provider {provider} stream ended without a terminal frame")
                    };
                    let terminal = synthesize_error(
                        last_partial.as_ref(),
                        &call.model,
                        provider,
                        &message,
                        usage,
                        now_ms(),
                    );
                    send_frame(&terminal);
                    let AssistantMessageEvent::Error { error } = terminal else {
                        unreachable!()
                    };
                    return Ok(respond_err(
                        StopReason::Error,
                        "transient",
                        message,
                        error.usage,
                    ));
                }
            }
        }

        // router::abort: synthesized done(aborted) carrying the partial.
        let terminal = synthesize_aborted(
            last_partial.as_ref(),
            &call.model,
            provider,
            last_usage,
            now_ms(),
        );
        send_frame(&terminal);
        let AssistantMessageEvent::Done { message } = terminal else {
            unreachable!()
        };
        Ok(ChatResponse {
            ok: true,
            provider: provider.to_string(),
            model: call.model.clone(),
            stop_reason: Some(StopReason::Aborted),
            usage: message.usage,
            error: None,
        })
    }
}

fn rand_unit() -> f64 {
    // uuid-derived cheap jitter: avoids pulling the rand crate for one knob
    (Uuid::new_v4().as_u128() % 1000) as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(code: &str) -> IIIError {
        IIIError::Remote {
            code: code.into(),
            message: "m".into(),
            stacktrace: None,
        }
    }

    #[test]
    fn unknown_function_maps_only_the_engines_function_not_found_code() {
        assert!(is_function_not_found(&remote("function_not_found")));
        assert!(is_function_not_found(&remote("FUNCTION_NOT_FOUND")));
        // the configuration worker's missing-entry code must NOT flip availability
        assert!(!is_function_not_found(&remote("NOT_FOUND")));
        assert!(!is_function_not_found(&IIIError::Timeout));
        assert!(is_router_coded(&remote("router/not_configured")));
        assert!(!is_router_coded(&remote("function_not_found")));
    }

    /// A pre-stream failure (here: a model that routes to no provider) must
    /// still emit exactly one terminal error frame to the sink. Without it,
    /// `router::complete`'s drain blocks for the full reader budget and
    /// `router::chat` streaming consumers never see a terminal.
    #[tokio::test]
    async fn pre_stream_routing_failure_emits_one_error_terminal_frame() {
        use crate::catalog::store::CatalogStore;
        use crate::chat::inflight::InflightMap;
        use crate::chat::relay::{ReadEvent, RelayRead};
        use crate::registry::store::RegistryStore;
        use crate::settings::RouterSettings;
        use crate::testkit::fake_channels::FakeChannel;
        use std::sync::RwLock;
        use std::time::Duration;

        // empty registry + empty catalog → nothing routes "ghost-model".
        let iii = iii_sdk::register_worker("ws://127.0.0.1:0", iii_sdk::InitOptions::default());
        let pipeline = ChatPipeline {
            iii: iii.clone(),
            registry: Arc::new(RegistryStore::new(iii.clone())),
            catalog: Arc::new(CatalogStore::new(iii.clone())),
            inflight: Arc::new(InflightMap::default()),
            settings: Arc::new(RwLock::new(RouterSettings::default())),
        };

        let ch = FakeChannel::new();
        let sink: Arc<dyn FrameSink> = Arc::new(ch.writer.clone());
        let call: ChatCall = serde_json::from_value(json!({
            "model": "ghost-model-routes-nowhere",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
        }))
        .unwrap();

        let result = pipeline.run(call, sink).await;
        assert!(
            result.is_err(),
            "routing failure still surfaces an error to the caller"
        );

        let mut reader = ch.reader;
        let first = reader.next(Duration::from_millis(50)).await;
        let ReadEvent::Msg(m) = first else {
            panic!("expected a terminal error frame on the channel, got {first:?}");
        };
        let ev: AssistantMessageEvent = serde_json::from_str(&m).unwrap();
        assert!(
            matches!(ev, AssistantMessageEvent::Error { .. }),
            "frame must be a terminal error, got {ev:?}"
        );
        let second = reader.next(Duration::from_millis(50)).await;
        assert!(
            matches!(second, ReadEvent::Eof | ReadEvent::Timeout),
            "exactly one frame expected, got a second: {second:?}"
        );
    }
}
