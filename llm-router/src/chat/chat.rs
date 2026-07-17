//! `router::chat` orchestration: validate → decide → gates → budget → attempt
//! loop (relay + invisible pre-first-frame retries) → exactly one terminal.
//! Each attempt invokes the provider's `provider::<id>::stream` iii function
//! via `iii.trigger` with a fresh router-owned iii channel.
//!
//! Engine-backed coverage: tests/integration.rs (happy path, cancellation,
//! abort, retry).
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::events::{AssistantMessageEvent, ErrorKind, StopReason};
use crate::types::router::{ChatResponse, ErrorShape};
use iii_helpers::observability::opentelemetry::trace::FutureExt as _;
use iii_sdk::channel::StreamChannelRef;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::catalog::queries::model_supports;
use crate::catalog::store::CatalogStore;
use crate::channels::create_router_channel;
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::store::RegistryStore;
use crate::routing::{decide, DecideInput};
use crate::settings::RouterSettings;
use crate::triggers::{self, RouterEvents};

use super::inflight::InflightMap;
use super::output_tokens::resolve_max_output_tokens;
use super::relay::{relay_frames, FrameSink, NoTerminalReason, RelayOpts, RelayResult};
use super::retry::backoff_ms;
use super::synthesize::{synthesize_aborted, synthesize_error};

/// ChatRequest minus writer_ref (the sink arrives separately — the function
/// handler wraps the looked-up writer, complete wraps its own).
///
/// `messages` / `tools` / `response_format` / `thinking_level` /
/// `provider_options` stay `Value`: they are forwarded to the provider verbatim,
/// so the router intentionally does not re-validate their shape. The struct
/// still derives `JsonSchema` so the SDK emits a real request schema (the
/// freeform sub-fields surface as permissive sub-schemas).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChatCall {
    #[serde(default)]
    pub request_id: Option<String>,
    pub model: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub messages: Value, // forwarded verbatim; validated to be an array
    #[serde(default)]
    pub tools: Option<Value>,
    #[serde(default)]
    pub response_format: Option<Value>,
    #[serde(default)]
    pub thinking_level: Option<Value>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub provider_options: Option<Value>,
    #[allow(dead_code)]
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Input of the `router::chat` iii function: a [`ChatCall`] plus the caller's
/// write channel. The handler relays assistant frames to `writer_ref` and also
/// returns the terminal [`ChatResponse`].
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChatFnInput {
    /// The caller's write channel (direction "write"); frames are relayed here.
    pub writer_ref: StreamChannelRef,
    #[serde(flatten)]
    pub call: ChatCall,
}

pub struct ChatPipeline {
    pub iii: IIIClient,
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub inflight: Arc<InflightMap>,
    pub config: ConfigCell,
    pub events: Arc<RouterEvents>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use crate::types::errors::is_function_not_found;

fn is_router_coded(err: &Error) -> bool {
    matches!(err, Error::Remote { code, .. } if code.starts_with("router/"))
}

impl ChatPipeline {
    pub async fn run(
        &self,
        call: ChatCall,
        sink: Arc<dyn FrameSink>,
    ) -> Result<ChatResponse, Error> {
        // A pre-stream failure must still leave exactly one terminal frame on
        // the sink. Without it, `router::complete`'s drain blocks for its full
        // reader budget and `router::chat` consumers never see a terminal.
        // (Regression: pre_stream_routing_failure_emits_one_error_terminal_frame.)
        let fail_pre_stream = |provider: &str, code: RouterCode, message: String| -> Error {
            // Pre-stream failures are permanent: a bad model or an unrouted
            // request won't succeed on retry. Mark the frame Permanent so a
            // streaming consumer inspecting error_kind doesn't retry it.
            let frame = synthesize_error(
                None,
                &call.model,
                provider,
                &message,
                ErrorKind::Permanent,
                None,
                now_ms(),
            );
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

        let config = snapshot(&self.config);
        let settings = config.settings().clone();
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

        let max_output_tokens = resolve_max_output_tokens(
            call.max_output_tokens,
            config
                .provider_slice(&provider)
                .and_then(|slice| slice.get("max_tokens"))
                .and_then(Value::as_u64),
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
                &request_id,
                sink,
            )
            .await;
        self.inflight.remove(&request_id);
        // Stamp token usage + cost onto the active `execute router::chat`
        // span. Every terminal outcome of the attempt loop funnels into the
        // ChatResponse, which carries the cost-filled usage.
        if let Ok(resp) = &result {
            super::telemetry::record_llm_call(
                &provider,
                &call.model,
                resp.stop_reason,
                resp.usage.as_ref(),
                resp.error.as_ref(),
            );
        }
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
        request_id: &str,
        sink: Arc<dyn FrameSink>,
    ) -> Result<ChatResponse, Error> {
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
            // router::abort closes the relay AND actively cancels the
            // provider's upstream via `provider::<id>::abort` — without it the
            // provider only notices the closed channel on its next (ping)
            // write, billing tokens meanwhile. Detached + best-effort: a
            // provider without the function (or a stale request id) is a
            // harmless error/no-op.
            inflight.set_closer({
                let close = reader.closer();
                let iii = self.iii.clone();
                let provider = provider.to_string();
                let rid = request_id.to_string();
                Arc::new(move || {
                    close();
                    let iii = iii.clone();
                    let function_id = format!("provider::{}::abort", provider);
                    let payload = serde_json::json!({ "request_id": rid });
                    tokio::spawn(async move {
                        let _ = iii
                            .trigger(TriggerRequest {
                                function_id,
                                payload,
                                action: None,
                                timeout_ms: Some(10_000),
                            })
                            .await;
                    });
                })
            });

            let stream_input = build_stream_input(
                call,
                provider,
                serde_json::to_value(&channel.writer_ref).expect("serializable writer_ref"),
                max_output_tokens,
                model_meta,
                request_id,
            );

            // The provider call runs concurrently with the relay; if it throws
            // pre-stream, closing the reader unblocks the loop immediately.
            let iii = self.iii.clone();
            let function_id = format!("provider::{provider}::stream");
            let closer = reader.closer();
            let timeout = settings.stream_timeout_ms;
            // Carry the caller's OTel context into the spawned task so the
            // provider stream nests under `router::chat` instead of rooting a
            // detached trace. `with_context` (not `cx.attach()`) because the
            // `ContextGuard` is `!Send` and can't cross the `.await` below.
            let parent_cx = iii_helpers::observability::opentelemetry::Context::current();
            let call_task = tokio::spawn(
                async move {
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
                }
                .with_context(parent_cx),
            );

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
                .unwrap_or(Err(Error::Handler("provider task panicked".into())));

            match relay {
                RelayResult::Done { terminal, .. } => {
                    let AssistantMessageEvent::Done { message } = terminal else {
                        unreachable!()
                    };
                    // A completed stream is definitive proof the provider is
                    // serving — heal a stale "down" flag (e.g. the boot-time
                    // reset in `RegistryStore::load`, or a past transient
                    // function_not_found) without waiting for a re-register.
                    if self.registry.set_availability(provider, true).await {
                        self.events
                            .emit(
                                triggers::PROVIDER_CHANGED,
                                json!({ "provider": provider, "op": "available" }),
                            )
                            .await;
                    }
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
                                    self.events
                                        .emit(
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
                        ErrorKind::Transient, // mid-stream no-terminal/idle: retryable
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

/// Build the per-attempt `provider::<id>::stream` payload (the wire shape of
/// `types::router::ProviderStreamInput`). Optional fields are omitted, never
/// null — provider-side schemas reject `null` where a string or array is
/// expected. `resolution_key` is the request id: stable across retry attempts
/// within a turn, fresh per turn, so providers can dedupe per-turn credential
/// resolution.
fn build_stream_input(
    call: &ChatCall,
    provider: &str,
    writer_ref: Value,
    max_output_tokens: u64,
    model_meta: Option<&crate::types::model::Model>,
    request_id: &str,
) -> Value {
    let mut input = serde_json::Map::new();
    input.insert("writer_ref".into(), writer_ref);
    input.insert("model".into(), Value::String(call.model.clone()));
    input.insert("messages".into(), call.messages.clone());
    input.insert("max_output_tokens".into(), json!(max_output_tokens));
    input.insert(
        "resolution_key".into(),
        Value::String(request_id.to_string()),
    );
    insert_present(
        &mut input,
        "system_prompt",
        call.system_prompt.clone().map(Value::String),
    );
    insert_present(&mut input, "tools", call.tools.clone());
    insert_present(&mut input, "response_format", call.response_format.clone());
    insert_present(&mut input, "thinking_level", call.thinking_level.clone());
    insert_present(
        &mut input,
        "provider_options",
        call.provider_options
            .as_ref()
            .and_then(|o| o.get(provider))
            .cloned(),
    );
    insert_present(
        &mut input,
        "model_meta",
        model_meta.and_then(|m| serde_json::to_value(m).ok()),
    );
    Value::Object(input)
}

fn insert_present(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        if !v.is_null() {
            map.insert(key.to_string(), v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(code: &str) -> Error {
        Error::Remote {
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
        assert!(!is_function_not_found(&Error::Timeout));
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
        use crate::config::state::new_config_cell;
        use crate::registry::store::RegistryStore;
        use crate::testkit::fake_channels::FakeChannel;
        use crate::triggers::RouterEvents;
        use std::time::Duration;

        // empty registry + empty catalog → nothing routes "ghost-model".
        let iii = iii_sdk::register_worker("ws://127.0.0.1:0", iii_sdk::InitOptions::default());
        let events = RouterEvents::register(&iii);
        let pipeline = ChatPipeline {
            iii: iii.clone(),
            registry: Arc::new(RegistryStore::new(iii.clone())),
            catalog: Arc::new(CatalogStore::new(iii.clone())),
            inflight: Arc::new(InflightMap::default()),
            config: new_config_cell(Value::Null),
            events,
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
        let AssistantMessageEvent::Error { error } = &ev else {
            panic!("frame must be a terminal error, got {ev:?}");
        };
        // A pre-stream failure is permanent — retrying won't fix a bad model or
        // an unrouted request. A `transient` kind would mislead a streaming
        // consumer that inspects error_kind into retrying.
        assert_eq!(
            error.error_kind,
            Some(crate::types::events::ErrorKind::Permanent),
            "pre-stream error frames must be permanent, not retryable"
        );
        let second = reader.next(Duration::from_millis(50)).await;
        assert!(
            matches!(second, ReadEvent::Eof | ReadEvent::Timeout),
            "exactly one frame expected, got a second: {second:?}"
        );
    }

    /// Provider-side schemas reject `null` where a string or array is
    /// expected, so absent options must be omitted keys, never null values.
    /// `resolution_key` must always be present and stable across the retry
    /// attempts of one request.
    #[test]
    fn stream_input_omits_absent_options_and_carries_resolution_key() {
        let call: ChatCall = serde_json::from_value(json!({
            "model": "claude-test",
            "messages": [],
        }))
        .unwrap();
        let writer_ref = json!({ "channel_id": "c", "access_key": "k", "direction": "write" });

        let attempt1 = build_stream_input(
            &call,
            "anthropic",
            writer_ref.clone(),
            32_000,
            None,
            "req-1",
        );
        let attempt2 = build_stream_input(
            &call,
            "anthropic",
            writer_ref.clone(),
            32_000,
            None,
            "req-1",
        );

        let obj = attempt1.as_object().unwrap();
        for absent in [
            "system_prompt",
            "tools",
            "response_format",
            "thinking_level",
            "provider_options",
            "model_meta",
        ] {
            assert!(
                !obj.contains_key(absent),
                "{absent} must be omitted, not null"
            );
        }
        assert!(
            obj.values().all(|v| !v.is_null()),
            "no null values on the wire"
        );
        assert_eq!(obj["resolution_key"], json!("req-1"));
        assert_eq!(obj["max_output_tokens"], json!(32_000));
        assert_eq!(
            attempt1["resolution_key"], attempt2["resolution_key"],
            "resolution_key is stable across attempts of one request"
        );

        // Present options ride through, and provider_options narrows to this
        // provider's slice.
        let call: ChatCall = serde_json::from_value(json!({
            "model": "claude-test",
            "messages": [],
            "system_prompt": "be brief",
            "tools": [{ "name": "t", "description": "d", "parameters": {} }],
            "thinking_level": "high",
            "provider_options": { "anthropic": { "beta": true }, "openai": { "x": 1 } },
        }))
        .unwrap();
        let input = build_stream_input(&call, "anthropic", writer_ref, 8192, None, "req-2");
        assert_eq!(input["system_prompt"], json!("be brief"));
        assert_eq!(input["thinking_level"], json!("high"));
        assert_eq!(input["provider_options"], json!({ "beta": true }));
    }
}
