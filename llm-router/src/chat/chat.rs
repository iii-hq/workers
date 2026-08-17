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
    /// Stable conversation identity, forwarded separately from the per-turn request id.
    #[serde(default)]
    pub session_id: Option<String>,
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

/// Dispatch failures that already carry a stable pre-stream error contract.
///
/// Provider typed-handler deserialization happens before the provider handler
/// can open its stream channel. Retrying `provider/invalid_request` cannot
/// succeed, and replacing it with a synthetic transient EOF error hides the
/// actionable cause from the caller.
fn terminal_dispatch_error(err: &Error) -> Option<(&str, ErrorKind)> {
    let Error::Remote { code, message, .. } = err else {
        return None;
    };
    if is_router_coded(err) {
        let kind = if code == RouterCode::ProviderUnavailable.as_str() {
            ErrorKind::Transient
        } else {
            ErrorKind::Permanent
        };
        return Some((message, kind));
    }
    (code == "provider/invalid_request").then_some((message, ErrorKind::Permanent))
}

type ProviderCallOutcome = Result<Value, Error>;
const MAX_PROVIDER_CALLS: usize = 64;

fn provider_call_slots() -> Arc<tokio::sync::Semaphore> {
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    Arc::clone(SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_PROVIDER_CALLS))))
}

enum ProviderCallAdmission {
    Admitted(tokio::sync::OwnedSemaphorePermit),
    Aborted,
    Saturated,
}

fn try_acquire_provider_call_slot(
    slots: Arc<tokio::sync::Semaphore>,
    aborted: &std::sync::atomic::AtomicBool,
) -> ProviderCallAdmission {
    if aborted.load(Ordering::SeqCst) {
        return ProviderCallAdmission::Aborted;
    }

    match slots.try_acquire_owned() {
        Ok(permit) => {
            if aborted.load(Ordering::SeqCst) {
                ProviderCallAdmission::Aborted
            } else {
                ProviderCallAdmission::Admitted(permit)
            }
        }
        Err(tokio::sync::TryAcquireError::NoPermits) if aborted.load(Ordering::SeqCst) => {
            ProviderCallAdmission::Aborted
        }
        Err(tokio::sync::TryAcquireError::NoPermits) => ProviderCallAdmission::Saturated,
        Err(tokio::sync::TryAcquireError::Closed) => {
            panic!("provider call semaphore is never closed")
        }
    }
}

fn pre_stream_error_kind(code: RouterCode) -> ErrorKind {
    match code {
        RouterCode::ProviderUnavailable => ErrorKind::Transient,
        _ => ErrorKind::Permanent,
    }
}

fn error_shape(
    code: RouterCode,
    kind: ErrorKind,
    message: String,
    detail: Option<String>,
) -> ErrorShape {
    ErrorShape {
        code: code.as_str().to_string(),
        message,
        kind: Some(kind),
        retryable: Some(kind.is_retryable()),
        detail,
    }
}

fn provider_error_presentation(
    kind: ErrorKind,
    provider: &str,
    model: &str,
) -> (RouterCode, String) {
    match kind {
        ErrorKind::AuthExpired => (
            RouterCode::ProviderAuthExpired,
            format!(
                "Authentication for provider \"{provider}\" needs attention. Update its credentials, then try again."
            ),
        ),
        ErrorKind::RateLimited => (
            RouterCode::ProviderRateLimited,
            format!("Provider \"{provider}\" is busy right now. Try again shortly."),
        ),
        ErrorKind::ContextOverflow => (
            RouterCode::ContextOverflow,
            format!(
                "This conversation is too large for model \"{model}\". Compact it, shorten the input, or choose a model with a larger context window."
            ),
        ),
        ErrorKind::Transient => (
            RouterCode::ProviderTransient,
            format!(
                "Provider \"{provider}\" temporarily failed while generating a response. Try again."
            ),
        ),
        ErrorKind::Permanent => (
            RouterCode::ProviderRejected,
            format!(
                "Provider \"{provider}\" rejected this request. Review the selected model and provider settings, then try again."
            ),
        ),
    }
}

fn take_provider_outcome(
    outcome_rx: &mut tokio::sync::oneshot::Receiver<ProviderCallOutcome>,
) -> Option<ProviderCallOutcome> {
    outcome_rx.try_recv().ok()
}

/// Release foreground ownership of a provider RPC once the relay has reached a
/// definitive result.
///
/// The RPC outcome is published before its error path closes the relay reader,
/// so a pre-stream dispatch failure remains available for classification. If
/// the RPC is still running, a reaper awaits it in the background instead of
/// aborting its `IIIClient::trigger` future: that future owns the SDK timeout
/// which removes the invocation from the SDK pending map. Thus Done, Error,
/// Idle, and Abort return immediately while cleanup remains bounded by the
/// `timeout_ms` passed to the trigger. The task itself owns an admission permit
/// acquired before the trigger is spawned, so the same process-wide bound
/// covers both foreground RPCs and background reapers.
///
/// This cannot repair the SDK's separate synchronous `send_message` failure
/// path, where `trigger` returns after inserting a pending invocation. That
/// cleanup belongs in `iii-sdk` itself.
async fn finish_provider_call(
    call_task: tokio::task::JoinHandle<()>,
    mut outcome_rx: tokio::sync::oneshot::Receiver<ProviderCallOutcome>,
) -> Option<ProviderCallOutcome> {
    let published = take_provider_outcome(&mut outcome_rx);
    if call_task.is_finished() {
        let joined = call_task.await;
        return published
            .or_else(|| take_provider_outcome(&mut outcome_rx))
            .or_else(|| {
                joined
                    .err()
                    .filter(|err| err.is_panic())
                    .map(|_| Err(Error::Handler("provider task panicked".into())))
            });
    }

    tokio::spawn(async move {
        if call_task.await.as_ref().is_err_and(|err| err.is_panic()) {
            eprintln!("[llm-router] provider stream task panicked after relay completion");
        }
    });
    published
}

#[allow(clippy::too_many_arguments)]
fn send_error_terminal(
    sink: &dyn FrameSink,
    partial: Option<&crate::types::messages::AssistantMessage>,
    model: &str,
    provider: &str,
    message: &str,
    error_kind: ErrorKind,
    usage: Option<crate::types::events::Usage>,
) -> AssistantMessageEvent {
    let frame = synthesize_error(
        partial,
        model,
        provider,
        message,
        error_kind,
        usage,
        now_ms(),
    );
    let _ = sink.send(&serde_json::to_string(&frame).expect("serializable frame"));
    frame
}

#[allow(clippy::too_many_arguments)]
fn fail_with_terminal(
    sink: &dyn FrameSink,
    partial: Option<&crate::types::messages::AssistantMessage>,
    model: &str,
    provider: &str,
    code: RouterCode,
    message: String,
    error_kind: ErrorKind,
    usage: Option<crate::types::events::Usage>,
) -> Error {
    send_error_terminal(sink, partial, model, provider, &message, error_kind, usage);
    RouterError::new(code, message).into()
}

fn provider_call_saturated_response(
    sink: &dyn FrameSink,
    partial: Option<&crate::types::messages::AssistantMessage>,
    model: &str,
    provider: &str,
    usage: Option<crate::types::events::Usage>,
) -> ChatResponse {
    let detail = format!("provider {provider} call capacity is saturated");
    let message = format!(
        "Provider \"{provider}\" is handling too many requests right now. Wait a moment and try again."
    );
    let terminal = send_error_terminal(
        sink,
        partial,
        model,
        provider,
        &message,
        ErrorKind::Transient,
        usage,
    );
    let AssistantMessageEvent::Error { error } = terminal else {
        unreachable!()
    };
    ChatResponse {
        ok: false,
        provider: provider.to_string(),
        model: model.to_string(),
        stop_reason: Some(StopReason::Error),
        usage: error.usage,
        error: Some(error_shape(
            RouterCode::CapacityExceeded,
            ErrorKind::Transient,
            message,
            Some(detail),
        )),
    }
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
            // Most routing/validation decisions are permanent. Availability is
            // topology state and may recover without changing the request.
            fail_with_terminal(
                sink.as_ref(),
                None,
                &call.model,
                provider,
                code,
                message,
                pre_stream_error_kind(code),
                None,
            )
        };

        // ── validate (pre-stream typed throws) ──
        if call.model.is_empty() {
            return Err(fail_pre_stream(
                "",
                RouterCode::InvalidRequest,
                "Select a model before sending the request.".into(),
            ));
        }
        if !call.messages.is_array() {
            return Err(fail_pre_stream(
                "",
                RouterCode::InvalidRequest,
                "The chat history is invalid. Start a new chat and try again.".into(),
            ));
        }

        let config = snapshot(&self.config);
        let settings = config.settings().clone();
        let provider_records = self.registry.list().await;
        let candidates = decide(&DecideInput {
            model: call.model.clone(),
            provider: call.provider.clone(),
            registered_providers: provider_records
                .iter()
                .map(|record| record.declaration.id.clone())
                .collect(),
            available_providers: provider_records
                .iter()
                .filter(|record| record.available)
                .map(|record| record.declaration.id.clone())
                .collect(),
            catalog: self.catalog.model_ids().await,
            heuristics: settings.routing_heuristics.clone(),
            default_provider: settings.default_provider.clone(),
        })
        .map_err(|e| fail_pre_stream("", e.code, e.message))?;
        let provider = candidates[0].clone(); // MVP consumes candidates[0]
        let record = match provider_records
            .into_iter()
            .find(|record| record.declaration.id == provider)
        {
            Some(record) => record,
            None => {
                return Err(fail_pre_stream(
                    &provider,
                    RouterCode::UnknownProvider,
                    format!(
                        "Provider \"{provider}\" is not registered. Choose a configured provider and try again."
                    ),
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
                        format!(
                            "Model \"{}\" does not support structured output. Choose a compatible model or disable structured output.",
                            call.model
                        ),
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
        let provider_generation = record.generation;

        let request_id = call
            .request_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let Some(entry_handle) = self.inflight.reserve(&request_id) else {
            return Err(fail_pre_stream(
                &provider,
                RouterCode::RequestInProgress,
                "This request is already running. Wait for it to finish or stop it before trying again."
                    .into(),
            ));
        };
        let result = self
            .attempts(
                &call,
                &provider,
                provider_generation,
                model_meta.as_ref(),
                max_output_tokens,
                &settings,
                &entry_handle,
                &request_id,
                sink,
            )
            .await;
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
        provider_generation: u64,
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
        let respond_err = |stop: StopReason,
                           code: RouterCode,
                           kind: ErrorKind,
                           message: String,
                           detail: Option<String>,
                           usage| ChatResponse {
            ok: false,
            provider: provider.to_string(),
            model: call.model.clone(),
            stop_reason: Some(stop),
            usage,
            error: Some(error_shape(code, kind, message, detail)),
        };

        let max_attempts = 1 + settings.retry_max;
        for attempt in 1..=max_attempts {
            if inflight.aborted.load(Ordering::SeqCst) {
                break;
            }
            // Admission is synchronous and happens before even allocating the
            // provider channel. Saturation cannot grow a hidden waiter queue or
            // create another SDK invocation/in-flight stream.
            let call_slot = match try_acquire_provider_call_slot(
                provider_call_slots(),
                inflight.aborted.as_ref(),
            ) {
                ProviderCallAdmission::Admitted(permit) => permit,
                ProviderCallAdmission::Aborted => break,
                ProviderCallAdmission::Saturated => {
                    return Ok(provider_call_saturated_response(
                        sink.as_ref(),
                        last_partial.as_ref(),
                        &call.model,
                        provider,
                        last_usage,
                    ));
                }
            };
            let channel = match create_router_channel(&self.iii).await {
                Ok(channel) => channel,
                Err(error) => {
                    tracing::error!(%error, provider, model = %call.model, "router channel creation failed");
                    let message =
                        "The response stream could not be started. Try again.".to_string();
                    return Err(fail_with_terminal(
                        sink.as_ref(),
                        last_partial.as_ref(),
                        &call.model,
                        provider,
                        RouterCode::StreamSetupFailed,
                        message,
                        ErrorKind::Transient,
                        last_usage,
                    ));
                }
            };
            let mut reader = channel.reader;
            if inflight.aborted.load(Ordering::SeqCst) {
                drop(call_slot);
                break;
            }
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
            // Holding the admission permit inside the call task makes the cap
            // a real bound on SDK pending stream invocations, including tasks
            // transferred to the post-terminal reaper.
            if inflight.aborted.load(Ordering::SeqCst) {
                drop(call_slot);
                break;
            }
            // Carry the caller's OTel context into the spawned task so the
            // provider stream nests under `router::chat` instead of rooting a
            // detached trace. `with_context` (not `cx.attach()`) because the
            // `ContextGuard` is `!Send` and can't cross the `.await` below.
            let parent_cx = iii_helpers::observability::opentelemetry::Context::current();
            let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
            let call_task = tokio::spawn(
                async move {
                    let _call_slot = call_slot;
                    let out = iii
                        .trigger(TriggerRequest {
                            function_id,
                            payload: stream_input,
                            action: None,
                            timeout_ms: Some(timeout),
                        })
                        .await;
                    let failed = out.is_err();
                    // Publish before closing: once the reader wakes on EOF the
                    // orchestration can classify function_not_found without
                    // waiting for (or racing) the provider task.
                    let _ = outcome_tx.send(out);
                    if failed {
                        closer();
                    }
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
            // Relay completion owns the foreground attempt lifecycle. Closing
            // the reader propagates cancellation to a cooperative provider;
            // the reaper lets a non-cooperative trigger run only until its SDK
            // timeout without delaying Done/Error/Idle/Abort.
            reader.close();
            let call_outcome = finish_provider_call(call_task, outcome_rx).await;

            match relay {
                RelayResult::Done { terminal, .. } => {
                    let AssistantMessageEvent::Done { message } = terminal else {
                        unreachable!()
                    };
                    // A completed stream is definitive proof the provider is
                    // serving — heal a stale "down" flag (e.g. the boot-time
                    // reset in `RegistryStore::load`, or a past transient
                    // function_not_found) without waiting for a re-register.
                    if self
                        .registry
                        .set_availability_if_current(provider, provider_generation, true)
                        .await
                    {
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
                        let delay = std::time::Duration::from_millis(backoff_ms(
                            attempt, 500, 8000, rand_unit,
                        ));
                        if inflight.wait_for_abort(delay).await {
                            break;
                        }
                        continue;
                    }
                    if !terminal_forwarded {
                        send_frame(&terminal);
                    }
                    let AssistantMessageEvent::Error { error } = terminal else {
                        unreachable!()
                    };
                    let kind = kind.unwrap_or(ErrorKind::Transient);
                    let detail = error
                        .error_message
                        .unwrap_or_else(|| "provider returned an error without details".into());
                    let (code, message) = provider_error_presentation(kind, provider, &call.model);
                    return Ok(respond_err(
                        StopReason::Error,
                        code,
                        kind,
                        message,
                        Some(detail),
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
                    if let Some(Err(err)) = call_outcome {
                        if !forwarded {
                            if let Some((message, kind)) = terminal_dispatch_error(&err) {
                                send_error_terminal(
                                    sink.as_ref(),
                                    last_partial.as_ref(),
                                    &call.model,
                                    provider,
                                    message,
                                    kind,
                                    usage,
                                );
                                return Err(err);
                            }
                            if is_function_not_found(&err) {
                                let message = format!(
                                    "Provider \"{provider}\" is temporarily unavailable. Try again or choose another provider."
                                );
                                let terminal_error = fail_with_terminal(
                                    sink.as_ref(),
                                    last_partial.as_ref(),
                                    &call.model,
                                    provider,
                                    RouterCode::ProviderUnavailable,
                                    message,
                                    ErrorKind::Transient,
                                    usage,
                                );
                                // Emit before availability persistence/event
                                // fan-out: neither side effect may hold the
                                // consumer's terminal frame hostage.
                                if self
                                    .registry
                                    .set_availability_if_current(
                                        provider,
                                        provider_generation,
                                        false,
                                    )
                                    .await
                                {
                                    self.events
                                        .emit(
                                            triggers::PROVIDER_CHANGED,
                                            json!({ "provider": provider, "op": "unavailable" }),
                                        )
                                        .await;
                                }
                                return Err(terminal_error);
                            }
                        }
                    }
                    let is_idle = matches!(reason, NoTerminalReason::Idle);
                    let can_retry = !forwarded
                        && !is_idle
                        && attempt < max_attempts
                        && !inflight.aborted.load(Ordering::SeqCst);
                    if can_retry {
                        let delay = std::time::Duration::from_millis(backoff_ms(
                            attempt, 500, 8000, rand_unit,
                        ));
                        if inflight.wait_for_abort(delay).await {
                            break;
                        }
                        continue;
                    }
                    let (code, message, detail) = if is_idle {
                        (
                            RouterCode::StreamIdleTimeout,
                            format!(
                                "Provider \"{provider}\" stopped responding before the answer completed. Try again."
                            ),
                            format!(
                                "provider {provider} stream idle past {}ms",
                                settings.idle_timeout_ms
                            ),
                        )
                    } else {
                        (
                            RouterCode::StreamIncomplete,
                            format!(
                                "Provider \"{provider}\" disconnected before completing the response. Try again."
                            ),
                            format!(
                                "provider {provider} stream ended without a terminal frame"
                            ),
                        )
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
                        code,
                        ErrorKind::Transient,
                        message,
                        Some(detail),
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
/// resolution. `session_id` remains stable across turns for provider affinity.
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
        "session_id",
        call.session_id.clone().map(Value::String),
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

    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

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
        assert_eq!(
            terminal_dispatch_error(&remote("provider/invalid_request")).map(|(_, kind)| kind),
            Some(ErrorKind::Permanent)
        );
        assert!(terminal_dispatch_error(&remote("provider/upstream_unavailable")).is_none());
    }

    #[test]
    fn only_provider_unavailable_is_a_transient_pre_stream_decision() {
        assert_eq!(
            pre_stream_error_kind(RouterCode::ProviderUnavailable),
            ErrorKind::Transient
        );
        for code in [
            RouterCode::InvalidRequest,
            RouterCode::UnknownProvider,
            RouterCode::NoProviderForModel,
            RouterCode::AmbiguousModel,
            RouterCode::NotConfigured,
            RouterCode::StructuredOutputUnsupported,
            RouterCode::RegistrationRejected,
            RouterCode::RequestInProgress,
            RouterCode::CapacityExceeded,
            RouterCode::StreamSetupFailed,
            RouterCode::StreamIdleTimeout,
            RouterCode::StreamIncomplete,
            RouterCode::ProviderAuthExpired,
            RouterCode::ProviderRateLimited,
            RouterCode::ContextOverflow,
            RouterCode::ProviderTransient,
            RouterCode::ProviderRejected,
        ] {
            assert_eq!(
                pre_stream_error_kind(code),
                ErrorKind::Permanent,
                "{} must remain permanent",
                code.as_str()
            );
        }
    }

    #[test]
    fn provider_errors_have_stable_codes_and_actionable_public_messages() {
        let cases = [
            (
                ErrorKind::AuthExpired,
                RouterCode::ProviderAuthExpired,
                "Update its credentials",
            ),
            (
                ErrorKind::RateLimited,
                RouterCode::ProviderRateLimited,
                "Try again shortly",
            ),
            (
                ErrorKind::ContextOverflow,
                RouterCode::ContextOverflow,
                "larger context window",
            ),
            (
                ErrorKind::Transient,
                RouterCode::ProviderTransient,
                "Try again",
            ),
            (
                ErrorKind::Permanent,
                RouterCode::ProviderRejected,
                "provider settings",
            ),
        ];

        for (kind, code, action) in cases {
            let (actual_code, message) = provider_error_presentation(kind, "openai", "gpt-5");
            assert_eq!(actual_code, code);
            assert!(message.contains(action), "missing action in {message}");
            assert!(!message.contains("terminal frame"));
        }
    }

    #[tokio::test]
    async fn provider_call_admission_slot_stays_held_while_the_rpc_is_reaped() {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let aborted = std::sync::atomic::AtomicBool::new(false);
        let ProviderCallAdmission::Admitted(call_slot) =
            try_acquire_provider_call_slot(slots.clone(), &aborted)
        else {
            panic!("first provider call must be admitted");
        };
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<ProviderCallOutcome>();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, mut dropped_rx) = tokio::sync::oneshot::channel();
        let call_task = tokio::spawn(async move {
            let _call_slot = call_slot;
            let _drop_signal = DropSignal(Some(dropped_tx));
            let _outcome_tx = outcome_tx;
            let _ = started_tx.send(());
            let _ = release_rx.await;
        });
        started_rx.await.unwrap();

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            finish_provider_call(call_task, outcome_rx),
        )
        .await
        .expect("relay completion must not wait for the provider RPC timeout");

        assert!(outcome.is_none());
        assert!(
            slots.clone().try_acquire_owned().is_err(),
            "a second SDK invocation must not be admitted while the reaper owns the first"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), &mut dropped_rx)
                .await
                .is_err(),
            "cleanup must not abort the SDK trigger future"
        );

        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("the background reaper must observe bounded task completion")
            .expect("drop signal sender must remain alive until task completion");
        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            slots.clone().acquire_owned(),
        )
        .await
        .expect("the admission slot must be released after SDK task completion")
        .expect("test semaphore remains open");
        drop(permit);
    }

    #[tokio::test]
    async fn saturated_provider_call_is_rejected_transiently_then_admitted_after_release() {
        use crate::chat::relay::{ReadEvent, RelayRead};
        use crate::testkit::fake_channels::FakeChannel;
        use std::time::Duration;

        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let aborted = std::sync::atomic::AtomicBool::new(false);
        let ProviderCallAdmission::Admitted(first_slot) =
            try_acquire_provider_call_slot(slots.clone(), &aborted)
        else {
            panic!("first provider call must be admitted");
        };
        assert!(
            matches!(
                try_acquire_provider_call_slot(slots.clone(), &aborted),
                ProviderCallAdmission::Saturated
            ),
            "a full limiter must reject synchronously rather than queue"
        );

        let ch = FakeChannel::new();
        let response =
            provider_call_saturated_response(&ch.writer, None, "busy-model", "busy-provider", None);
        assert!(!response.ok);
        assert_eq!(response.stop_reason, Some(StopReason::Error));
        let error = response.error.as_ref().unwrap();
        assert_eq!(error.code, RouterCode::CapacityExceeded.as_str());
        assert_eq!(error.kind, Some(ErrorKind::Transient));
        assert_eq!(error.retryable, Some(true));
        assert!(error.message.contains("try again"));
        assert!(error.detail.as_deref().unwrap().contains("capacity"));

        let mut reader = ch.reader;
        let ReadEvent::Msg(frame) = reader.next(Duration::from_millis(50)).await else {
            panic!("saturation must emit a terminal error frame");
        };
        let AssistantMessageEvent::Error { error } =
            serde_json::from_str::<AssistantMessageEvent>(&frame).unwrap()
        else {
            panic!("saturation terminal must be an error");
        };
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        assert!(matches!(
            reader.next(Duration::from_millis(25)).await,
            ReadEvent::Timeout
        ));

        aborted.store(true, Ordering::SeqCst);
        assert!(matches!(
            try_acquire_provider_call_slot(slots.clone(), &aborted),
            ProviderCallAdmission::Aborted
        ));
        aborted.store(false, Ordering::SeqCst);

        drop(first_slot);
        let ProviderCallAdmission::Admitted(_next_slot) =
            try_acquire_provider_call_slot(slots, &aborted)
        else {
            panic!("released capacity must admit the next provider call");
        };
    }

    #[tokio::test]
    async fn dispatch_error_published_before_reader_close_survives_task_cleanup() {
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<ProviderCallOutcome>();
        let (published_tx, published_rx) = tokio::sync::oneshot::channel();
        let call_task = tokio::spawn(async move {
            let _ = outcome_tx.send(Err(remote("function_not_found")));
            let _ = published_tx.send(());
        });
        published_rx.await.unwrap();

        let outcome = finish_provider_call(call_task, outcome_rx)
            .await
            .expect("published outcome must be retained");
        assert!(is_function_not_found(&outcome.unwrap_err()));
    }

    #[tokio::test]
    async fn provider_unavailable_failure_emits_exactly_one_terminal_error() {
        use crate::chat::relay::{ReadEvent, RelayRead};
        use crate::testkit::fake_channels::FakeChannel;
        use std::time::Duration;

        let ch = FakeChannel::new();
        let err = fail_with_terminal(
            &ch.writer,
            None,
            "ghost-model",
            "ghost",
            RouterCode::ProviderUnavailable,
            "provider ghost unavailable".into(),
            ErrorKind::Transient,
            None,
        );
        assert!(matches!(
            err,
            Error::Remote { ref code, .. } if code == "router/provider_unavailable"
        ));

        let mut reader = ch.reader;
        let ReadEvent::Msg(frame) = reader.next(Duration::from_millis(50)).await else {
            panic!("expected terminal error frame");
        };
        let event: AssistantMessageEvent = serde_json::from_str(&frame).unwrap();
        let AssistantMessageEvent::Error { error } = event else {
            panic!("provider_unavailable must emit an error terminal");
        };
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        assert_eq!(
            error.error_message.as_deref(),
            Some("provider ghost unavailable")
        );
        assert!(matches!(
            reader.next(Duration::from_millis(25)).await,
            ReadEvent::Timeout
        ));
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
            "session_id",
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
            "session_id": "s_1",
            "provider_options": { "anthropic": { "beta": true }, "openai": { "x": 1 } },
        }))
        .unwrap();
        let input = build_stream_input(&call, "anthropic", writer_ref, 8192, None, "req-2");
        assert_eq!(input["system_prompt"], json!("be brief"));
        assert_eq!(input["thinking_level"], json!("high"));
        assert_eq!(input["session_id"], json!("s_1"));
        assert_eq!(input["provider_options"], json!({ "beta": true }));
    }
}
