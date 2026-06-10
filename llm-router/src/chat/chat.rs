//! `router::chat` orchestration: validate → decide → gates → budget → attempt
//! loop (relay + invisible pre-first-frame retries) → exactly one terminal.
//! Each attempt invokes the provider's `provider::<id>::stream` iii function
//! over the bus with a fresh router-owned iii channel.
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use crate::types::errors::{RouterCode, RouterError};
use crate::types::events::{AssistantMessageEvent, StopReason};
use crate::types::router::{ChatResponse, ErrorShape};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::bus::{Bus, BusError};
use crate::catalog::queries::model_supports;
use crate::catalog::store::CatalogStore;
use crate::config::entry::read_entry_value;
use crate::registry::store::RegistryStore;
use crate::routing::{decide, DecideInput};
use crate::settings::{provider_slices, RouterSettings};
use crate::triggers::TriggerEmitter;

use super::inflight::InflightMap;
use super::output_tokens::resolve_max_output_tokens;
use super::relay::{
    relay_frames, ChannelFactory, FrameSink, NoTerminalReason, RelayOpts, RelayResult,
};
use super::retry::backoff_ms;
use super::synthesize::{synthesize_aborted, synthesize_error};

/// ChatRequest minus writer_ref (the sink arrives separately — the function
/// handler wraps the hydrated/looked-up writer, complete wraps its own).
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
    pub bus: Arc<dyn Bus>,
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub emitter: TriggerEmitter,
    pub inflight: Arc<InflightMap>,
    pub settings: Arc<RwLock<RouterSettings>>,
    pub channels: Arc<dyn ChannelFactory>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl ChatPipeline {
    pub async fn run(
        &self,
        call: ChatCall,
        sink: Arc<dyn FrameSink>,
    ) -> Result<ChatResponse, BusError> {
        // ── validate (pre-stream typed throws) ──
        if call.model.is_empty() {
            return Err(RouterError::new(RouterCode::InvalidRequest, "model is required").into());
        }
        if !call.messages.is_array() {
            return Err(
                RouterError::new(RouterCode::InvalidRequest, "messages must be an array").into(),
            );
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
        .map_err(BusError::from)?;
        let provider = candidates[0].clone(); // MVP consumes candidates[0]
        let record = self.registry.get(&provider).await.ok_or_else(|| {
            BusError::from(RouterError::new(
                RouterCode::UnknownProvider,
                format!("unknown provider {provider}"),
            ))
        })?;

        // Structured-output gate: known model without the flag throws; unknown
        // model fails open — the provider is the final arbiter.
        let model_meta = self.catalog.get(&provider, &call.model).await;
        if call.response_format.is_some() {
            if let Some(meta) = &model_meta {
                if !model_supports(meta, "structured_output") {
                    return Err(RouterError::new(
                        RouterCode::StructuredOutputUnsupported,
                        format!("structured output unsupported for model {}", call.model),
                    )
                    .into());
                }
            }
        }

        let entry = read_entry_value(&self.bus).await;
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
                &record,
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
        record: &crate::registry::store::ProviderRecord,
        model_meta: Option<&crate::types::model::Model>,
        max_output_tokens: u64,
        settings: &RouterSettings,
        inflight: &super::inflight::InflightEntry,
        sink: Arc<dyn FrameSink>,
    ) -> Result<ChatResponse, BusError> {
        let pricing = model_meta.and_then(|m| m.pricing.clone());
        let mut last_partial = None;
        let mut last_usage = None;
        let _ = record;

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
            let channel = self.channels.create().await?;
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
            let bus = self.bus.clone();
            let function_id = format!("provider::{provider}::stream");
            let closer = reader.closer();
            let timeout = settings.stream_timeout_ms;
            let call_task = tokio::spawn(async move {
                let out = bus.trigger(&function_id, stream_input, Some(timeout)).await;
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
                .unwrap_or(Err(BusError::Transport("provider task panicked".into())));

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
                            match &err {
                                BusError::Coded { code, .. } if code.starts_with("router/") => {
                                    return Err(err)
                                }
                                BusError::FunctionNotFound(_) => {
                                    if self.registry.set_availability(provider, false).await {
                                        self.emitter
                                            .emit(
                                                "router::provider::changed",
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
                                _ => {}
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
    use crate::catalog::store::CatalogStore;
    use crate::chat::inflight::InflightMap;
    use crate::chat::relay::{ChannelFactory, ReadEvent, RelayRead};
    use crate::config::entry::EntryWriteLock;
    use crate::registry::register::make_provider_register;
    use crate::registry::store::RegistryStore;
    use crate::settings::RouterSettings;
    use crate::testkit::fake_bus::FakeBus;
    use crate::testkit::fake_channels::{ChannelHub, FakeChannel};
    use crate::triggers::TriggerEmitter;
    use crate::types::events::AssistantMessageEvent;
    use serde_json::json;
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    struct H {
        bus: Arc<FakeBus>,
        hub: Arc<ChannelHub>,
        registry: Arc<RegistryStore>,
        inflight: Arc<InflightMap>,
        chat: Arc<ChatPipeline>,
    }

    async fn harness() -> H {
        let bus = FakeBus::new();
        let hub = ChannelHub::new();
        let registry = Arc::new(RegistryStore::new(bus.clone()));
        let catalog = Arc::new(CatalogStore::new(bus.clone()));
        let emitter = TriggerEmitter::install(bus.clone() as Arc<dyn crate::bus::Bus>, &*bus);
        let register = make_provider_register(
            bus.clone(),
            registry.clone(),
            catalog.clone(),
            emitter.clone(),
            EntryWriteLock::default(),
        );
        register(json!({
            "id": "fake", "worker_id": "w-fake", "defaults": { "max_tokens": 8192 },
            "models": [{
                "id": "m1", "provider": "fake", "context_window": 200000, "max_output_tokens": 64000,
                "supports_tools": true, "pricing": { "input": 3.0, "output": 15.0 }
            }]
        }))
        .await
        .unwrap();
        registry.set_availability("fake", true).await;
        let settings = RouterSettings {
            idle_timeout_ms: 200,
            ..Default::default()
        };
        let inflight = Arc::new(InflightMap::default());
        let chat = Arc::new(ChatPipeline {
            bus: bus.clone(),
            registry: registry.clone(),
            catalog,
            emitter,
            inflight: inflight.clone(),
            settings: Arc::new(RwLock::new(settings)),
            channels: hub.clone() as Arc<dyn ChannelFactory>,
        });
        H {
            bus,
            hub,
            registry,
            inflight,
            chat,
        }
    }

    /// Register a scripted provider::fake::stream that looks up the live
    /// writer via the hub and runs `script(input, writer)`.
    fn script<F, Fut>(h: &H, f: F)
    where
        F: Fn(serde_json::Value, crate::testkit::fake_channels::FakeWriter) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<serde_json::Value, crate::bus::BusError>>
            + Send
            + 'static,
    {
        let hub = h.hub.clone();
        let f = Arc::new(f);
        h.bus.register_function(
            "provider::fake::stream",
            crate::bus::handler(move |input: serde_json::Value| {
                let hub = hub.clone();
                let f = f.clone();
                async move {
                    let r: crate::types::channel::StreamChannelRef =
                        serde_json::from_value(input["writer_ref"].clone()).unwrap();
                    let writer = hub.writer_for(&r).expect("forwarded ref resolves");
                    f(input, writer).await
                }
            }),
        );
    }

    fn done_frame(model: &str, usage_in: u64, usage_out: u64) -> String {
        let mut m = crate::chat::synthesize::empty_partial(model, "fake", 2);
        m.usage = Some(crate::types::events::Usage {
            input: Some(usage_in),
            output: Some(usage_out),
            ..Default::default()
        });
        serde_json::to_string(&AssistantMessageEvent::Done { message: m }).unwrap()
    }

    fn request(over: serde_json::Value) -> serde_json::Value {
        let mut req = json!({ "model": "m1", "messages": [] });
        req.as_object_mut()
            .unwrap()
            .extend(over.as_object().cloned().unwrap_or_default());
        req
    }

    async fn run(
        h: &H,
        req: serde_json::Value,
    ) -> (
        Result<crate::types::router::ChatResponse, crate::bus::BusError>,
        Vec<AssistantMessageEvent>,
    ) {
        let mut caller = FakeChannel::new();
        let sink: Arc<dyn crate::chat::relay::FrameSink> = Arc::new(caller.writer.clone());
        let res = h.chat.run(serde_json::from_value(req).unwrap(), sink).await;
        let mut frames = vec![];
        while let ReadEvent::Msg(m) = caller.reader.next(Duration::from_millis(50)).await {
            frames.push(serde_json::from_str(&m).unwrap());
        }
        (res, frames)
    }

    #[tokio::test(start_paused = true)]
    async fn happy_path_streams_clamps_budget_and_fills_cost() {
        let h = harness().await;
        let seen = Arc::new(std::sync::Mutex::new(json!(null)));
        let s2 = seen.clone();
        script(&h, move |input, writer| {
            let s = s2.clone();
            async move {
                *s.lock().unwrap() = input.clone();
                writer
                    .send(&serde_json::to_string(&AssistantMessageEvent::Ping).unwrap())
                    .unwrap();
                writer
                    .send(&done_frame(input["model"].as_str().unwrap(), 10, 5))
                    .unwrap();
                writer.close();
                Ok(json!({ "ok": true }))
            }
        });
        let (res, frames) = run(
            &h,
            request(
                json!({ "provider_options": { "fake": { "beta": true }, "other": { "x": 1 } } }),
            ),
        )
        .await;
        let res = res.unwrap();
        assert!(res.ok);
        assert_eq!(res.provider, "fake");
        let cost = res.usage.unwrap().cost_usd.unwrap();
        assert!((cost - (10.0 * 3.0 + 5.0 * 15.0) / 1e6).abs() < 1e-12);
        assert!(matches!(
            frames.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
        let input = seen.lock().unwrap().clone();
        assert_eq!(input["max_output_tokens"], 32_000); // min(64k ceiling, 32k soft cap)
        assert_eq!(input["provider_options"], json!({ "beta": true })); // routed slice only
        assert_eq!(input["model_meta"]["id"], "m1");
    }

    #[tokio::test(start_paused = true)]
    async fn pre_stream_throws_routing_validation_and_structured_output_gate() {
        let h = harness().await;
        let (res, _) = run(&h, request(json!({ "provider": "typo" }))).await;
        assert_eq!(res.unwrap_err().code(), Some("router/unknown_provider"));
        let (res, _) = run(&h, json!({ "model": "", "messages": [] })).await;
        assert_eq!(res.unwrap_err().code(), Some("router/invalid_request"));
        let (res, _) = run(
            &h,
            request(json!({ "response_format": { "type": "json" } })),
        )
        .await;
        assert_eq!(
            res.unwrap_err().code(),
            Some("router/structured_output_unsupported")
        );
        // unknown model + explicit provider: cold-catalog route, fail-open on the capability
        script(&h, |_, writer| async move {
            writer.send(&done_frame("brand-new", 1, 1)).unwrap();
            writer.close();
            Ok(json!({ "ok": true }))
        });
        let (res, _) = run(
            &h,
            json!({ "model": "brand-new", "provider": "fake", "messages": [], "response_format": { "type": "json" } }),
        )
        .await;
        assert!(res.unwrap().ok);
    }

    #[tokio::test(start_paused = true)]
    async fn pre_first_frame_429_retries_invisibly_mid_stream_errors_never_retry() {
        let h = harness().await;
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c2 = calls.clone();
        script(&h, move |input, writer| {
            let calls = c2.clone();
            async move {
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    let mut err = crate::chat::synthesize::empty_partial("m1", "fake", 1);
                    err.stop_reason = crate::types::events::StopReason::Error;
                    err.error_kind = Some(crate::types::events::ErrorKind::RateLimited);
                    writer
                        .send(
                            &serde_json::to_string(&AssistantMessageEvent::Error { error: err })
                                .unwrap(),
                        )
                        .unwrap();
                } else {
                    writer
                        .send(&done_frame(input["model"].as_str().unwrap(), 1, 1))
                        .unwrap();
                }
                writer.close();
                Ok(json!({ "ok": true }))
            }
        });
        let (res, frames) = run(&h, request(json!({}))).await;
        assert!(res.unwrap().ok);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(frames.len(), 1); // the caller saw one clean stream, no 429
    }

    #[tokio::test(start_paused = true)]
    async fn crash_without_terminal_retries_then_synthesizes_transient_error() {
        let h = harness().await;
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c2 = calls.clone();
        script(&h, move |_, writer| {
            let calls = c2.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                writer.close(); // crash: no frames at all
                Ok(json!({ "ok": false }))
            }
        });
        let (res, frames) = run(&h, request(json!({}))).await;
        let res = res.unwrap();
        assert!(!res.ok);
        assert_eq!(res.error.unwrap().code, "transient");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1 + RouterSettings::default().retry_max
        );
        let Some(AssistantMessageEvent::Error { error }) = frames.last() else {
            panic!()
        };
        assert!(error.error_message.as_ref().unwrap().contains("fake"));
    }

    #[tokio::test(start_paused = true)]
    async fn provider_throws_propagate_and_function_not_found_flips_availability() {
        let h = harness().await;
        h.bus.register_function(
            "provider::fake::stream",
            crate::bus::handler(|_| async move {
                Err(crate::bus::BusError::Coded {
                    code: "router/not_configured".into(),
                    message: "no key".into(),
                })
            }),
        );
        let (res, _) = run(&h, request(json!({}))).await;
        assert_eq!(res.unwrap_err().code(), Some("router/not_configured"));

        h.bus.set_unavailable("provider::fake::stream");
        let (res, _) = run(&h, request(json!({}))).await;
        assert_eq!(res.unwrap_err().code(), Some("router/provider_unavailable"));
        assert!(!h.registry.get("fake").await.unwrap().available);
    }

    #[tokio::test(start_paused = true)]
    async fn abort_mid_stream_synthesizes_done_aborted() {
        let h = harness().await;
        script(&h, |_, writer| async move {
            let mut start = crate::chat::synthesize::empty_partial("m1", "fake", 1);
            start
                .content
                .push(crate::types::content::ContentBlock::Text { text: "Hel".into() });
            writer
                .send(
                    &serde_json::to_string(&AssistantMessageEvent::Start { partial: start })
                        .unwrap(),
                )
                .unwrap();
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                if writer
                    .send(&serde_json::to_string(&AssistantMessageEvent::Ping).unwrap())
                    .is_err()
                {
                    return Ok(json!({ "ok": true, "status": "aborted" })); // upstream abort observed
                }
            }
        });
        let mut caller = FakeChannel::new();
        let sink: Arc<dyn crate::chat::relay::FrameSink> = Arc::new(caller.writer.clone());
        let req = serde_json::from_value(request(json!({ "request_id": "abort-me" }))).unwrap();
        let chat = h.chat.clone();
        let pending = tokio::spawn(async move { chat.run(req, sink).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(h.inflight.abort("abort-me"));
        let res = pending.await.unwrap().unwrap();
        assert!(res.ok);
        assert_eq!(
            res.stop_reason,
            Some(crate::types::events::StopReason::Aborted)
        );
        let mut last = None;
        while let ReadEvent::Msg(m) = caller.reader.next(Duration::from_millis(50)).await {
            last = Some(serde_json::from_str::<AssistantMessageEvent>(&m).unwrap());
        }
        let Some(AssistantMessageEvent::Done { message }) = last else {
            panic!()
        };
        assert_eq!(
            message.stop_reason,
            crate::types::events::StopReason::Aborted
        );
        assert_eq!(message.content.len(), 1); // partial survived
    }
}
