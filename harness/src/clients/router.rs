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
use tokio::sync::{watch, Notify};

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
    pub max_output_tokens: Option<u64>,
    pub thinking_level: Option<ThinkingLevel>,
    pub provider_options: Option<Value>,
}

/// The assembled result of one stream.
pub struct ChatOutcome {
    pub message: AssistantMessage,
    pub ok: bool,
    pub stop_reason: Option<StopReason>,
    pub error: Option<String>,
}

/// Ceiling for `count_tokens`, independent of the generation-sized router
/// timeout. Counting is a tokenizer pass, not a model call, and it runs on the
/// path that finalizes a turn: inheriting a timeout measured in minutes would
/// let one slow counting endpoint hold `turn-completed` open for as long as a
/// whole generation. A count that has not answered by now is not worth the
/// wait — the caller keeps its estimate.
const COUNT_TOKENS_TIMEOUT_MS: u64 = 10_000;

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
    ///
    /// `abort_rx` is the harness-local cancel backstop: `router::abort` is the
    /// happy path (the router closes the stream → EOF), but it's a no-op when
    /// it races ahead of the router's stream registration or after a router
    /// restart — then only this local signal cuts the await (which otherwise
    /// blocks up to `router_timeout_ms`).
    pub async fn chat(
        &self,
        params: ChatParams,
        sink: &dyn StreamSink,
        mut abort_rx: watch::Receiver<bool>,
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

        let request_id = params.request_id.clone();
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
        if let Some(max_output_tokens) = params.max_output_tokens {
            payload["max_output_tokens"] = json!(max_output_tokens);
        }
        if let Some(tl) = &params.thinking_level {
            payload["thinking_level"] = serde_json::to_value(tl).unwrap_or(Value::Null);
        }
        if let Some(options) = &params.provider_options {
            payload["provider_options"] = options.clone();
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
        let mut trigger = tokio::spawn(
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
        // Raw in-flight tool-call arguments per call id: providers degrade
        // incomplete args to a placeholder object (replay safety), so the
        // delta text accumulated here is the only live view of a long
        // arguments stream — injected into coalesced partials as
        // `_streaming` so UIs can show the command being formed.
        let mut args_acc: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // Cumulative message across slim delta frames (boundary snapshot +
        // open-block deltas); fat frames just refresh the snapshot.
        let mut tracker = PartialTracker::default();

        // Consume frames until reader EOF — but ALSO watch the held-open
        // trigger: when it resolves, the router is done (ack) or the dispatch
        // failed before a writer ever attached (e.g. provider unavailable).
        // In the failure case the channel never EOFs, so waiting on frames
        // alone would hang this turn forever with the session stuck "working".
        // After the trigger resolves, drain already-buffered frames briefly,
        // then fall through to the outcome handling below.
        let mut response: Option<Result<Value, iii_sdk::errors::Error>> = None;
        let mut drain_deadline = Instant::now();
        // Resolves when harness::stop fires the turn's local cancel; a dropped
        // sender (registry cleared — no stop can arrive) parks forever instead
        // of busy-looping the select.
        let abort_fired = async move {
            if abort_rx.wait_for(|a| *a).await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(abort_fired);
        loop {
            let frame = if response.is_some() {
                match tokio::time::timeout(Duration::from_millis(750), rx.recv()).await {
                    Ok(f) => f,
                    // A lull after the ack means "no writer is coming" only
                    // for failed dispatches. A successful response is always
                    // followed by a terminal frame, so keep draining for it —
                    // treating the lull as EOF would synthesize a no-terminal
                    // error over an ok stream (the forwarder can lag the ack).
                    // ponytail: 10s cap; a writer lagging past it is a wedge.
                    Err(_) => {
                        let ok_ack = matches!(&response, Some(Ok(v))
                            if v.get("ok").and_then(Value::as_bool).unwrap_or(true));
                        if ok_ack && final_message.is_none() && Instant::now() < drain_deadline {
                            continue;
                        }
                        None
                    }
                }
            } else {
                tokio::select! {
                    f = rx.recv() => f,
                    // Local cancel backstop: cut the await now, return the
                    // partial as Aborted. No arm in the post-ack drain branch
                    // above — that path is bounded (≤10s) already.
                    _ = &mut abort_fired => {
                        // notify_one, not notify_waiters: it stores a permit
                        // when the pump task hasn't polled its notified() yet
                        // (a pre-fired cancel can get here before the spawned
                        // pump first runs), so the wakeup can't be lost and
                        // pump.await can't hang on next_binary().
                        cancel.notify_one();
                        let _ = pump.await;
                        // Kill the held-open router::chat task; the router's
                        // own teardown was already requested via router::abort.
                        trigger.abort();
                        // Re-request the router abort: stop.rs's router::abort
                        // can race ahead of the router's stream registration
                        // and no-op — by now the stream is certainly
                        // registered, so this lands and the provider stops
                        // generating instead of streaming into the closed
                        // channel until its own terminal. Detached: an abort
                        // against a hung router must not block the stop path.
                        let client = self.clone();
                        let rid = request_id.clone();
                        tokio::spawn(async move {
                            client.abort(&rid).await;
                        });
                        // Prefer a terminal frame's complete message over the
                        // tracker's partial: a stop landing between a Done
                        // frame and EOF must not replace the full assistant
                        // entry with the stale pre-Done partial.
                        let mut message = final_message
                            .or_else(|| tracker.current())
                            .unwrap_or_else(|| {
                                empty_assistant(
                                    params.provider.as_deref().unwrap_or(""),
                                    &params.model,
                                )
                            });
                        message.stop_reason = StopReason::Aborted;
                        return Ok(ChatOutcome {
                            ok: false,
                            stop_reason: Some(StopReason::Aborted),
                            message,
                            error: None,
                        });
                    }
                    r = &mut trigger => {
                        response = Some(r.map_err(|e| {
                            HarnessError::Internal(format!("router::chat task: {e}"))
                        })?);
                        drain_deadline = Instant::now() + Duration::from_secs(10);
                        continue;
                    }
                }
            };
            let Some(frame) = frame else { break };
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
                    if let AssistantMessageEvent::FunctioncallDelta { partial, delta, id } = &other
                    {
                        // Pre-id producers: guess from the frame's own fat
                        // partial or, on slim frames, from the last boundary
                        // snapshot (FunctioncallStart carries the call block).
                        let id = if id.is_empty() {
                            partial
                                .as_ref()
                                .or(tracker.base.as_ref())
                                .and_then(open_call_id)
                                .map(str::to_string)
                        } else {
                            Some(id.clone())
                        };
                        if let Some(id) = id {
                            args_acc.entry(id).or_default().push_str(delta);
                        }
                    }
                    tracker.apply(&other);
                    if other.is_content() && last_emit.elapsed() >= coalesce {
                        if let Some(cum) = tracker.current() {
                            match enrich_streaming_args(&cum, &args_acc) {
                                Some(enriched) => sink.on_update(&enriched).await,
                                None => sink.on_update(&cum).await,
                            }
                            last_emit = Instant::now();
                        }
                    }
                }
            }
        }

        // Stream drained; stop the pump and collect the trigger ack.
        // notify_one: latched — safe even if the pump task hasn't polled yet.
        cancel.notify_one();
        let _ = pump.await;
        let response = match response {
            Some(r) => r,
            None => trigger
                .await
                .map_err(|e| HarnessError::Internal(format!("router::chat task: {e}")))?,
        };

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
            // A dispatch/ack rejection (e.g. provider unavailable) is
            // authoritative — retrying in-turn cannot help. Only a
            // frame-less-but-acked stream stays classified transient.
            m.error_kind = Some(if response_error.is_some() {
                crate::types::event::ErrorKind::Permanent
            } else {
                crate::types::event::ErrorKind::Transient
            });
            m
        });

        let stop_reason = Some(message.stop_reason);
        // A frame-less-but-acked stream synthesizes an Error message above
        // with no terminal_error/response_error set — derive the outcome
        // error from it so the turn fails instead of completing "ok" around
        // an empty error-stopped message.
        let error = terminal_error
            .or(response_error)
            .or_else(|| {
                (message.stop_reason == StopReason::Error).then(|| {
                    message
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "router produced no terminal frame".to_string())
                })
            })
            .filter(|_| !ok || message.stop_reason == StopReason::Error);
        Ok(ChatOutcome {
            ok: ok && message.stop_reason != StopReason::Error && error.is_none(),
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

    /// Exact token count for a request shape via the resolved provider's
    /// tokenizer (`None` when the router is absent, the provider has no
    /// counter, or the call fails — the caller falls back to its estimate).
    /// Returns `(tokens, estimator)`. Counting never runs the model and
    /// never enters the session context.
    pub async fn count_tokens(
        &self,
        model: &str,
        provider: Option<&str>,
        system_prompt: Option<&str>,
        tools: Option<&[AgentFunction]>,
        messages: &[Value],
    ) -> Option<(u64, String)> {
        let mut payload = json!({ "model": model, "messages": messages });
        if let Some(p) = provider {
            payload["provider"] = json!(p);
        }
        if let Some(sp) = system_prompt {
            payload["system_prompt"] = json!(sp);
        }
        if let Some(t) = tools {
            payload["tools"] = serde_json::to_value(t).ok()?;
        }
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::count_tokens".into(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms.min(COUNT_TOKENS_TIMEOUT_MS)),
            })
            .await
            .ok()?;
        let tokens = resp.get("tokens").and_then(Value::as_u64)?;
        let estimator = resp
            .get("estimator")
            .and_then(Value::as_str)
            .unwrap_or("provider")
            .to_string();
        Some((tokens, estimator))
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

/// Cumulative-message view over the streamed frames: the last
/// block-boundary snapshot plus the locally accumulated deltas of the open
/// text/thinking block. Slim delta frames carry no `partial`, so this
/// tracker is the only live view of the message between block boundaries;
/// legacy fat deltas (partial present) simply replace the snapshot, which
/// preserves the old behavior byte for byte. In-flight function-call
/// arguments intentionally stay in `args_acc` (`enrich_streaming_args`).
#[derive(Default)]
struct PartialTracker {
    base: Option<AssistantMessage>,
    open: Option<OpenBlock>,
}

enum OpenBlock {
    Text { seed: String, acc: String },
    Thinking { seed: String, acc: String },
}

impl PartialTracker {
    fn apply(&mut self, event: &AssistantMessageEvent) {
        use crate::types::content::ContentBlock;
        match event {
            // Block boundaries carry authoritative snapshots (thinking
            // signatures and finalized call args exist only here).
            AssistantMessageEvent::Start { partial }
            | AssistantMessageEvent::TextEnd { partial }
            | AssistantMessageEvent::ThinkingEnd { partial }
            | AssistantMessageEvent::FunctioncallStart { partial }
            | AssistantMessageEvent::FunctioncallEnd { partial } => {
                self.base = Some(partial.clone());
                self.open = None;
            }
            // Producers may or may not include the just-opened (empty)
            // block in the Start snapshot — pop a trailing match into the
            // seed so the merge never duplicates it.
            AssistantMessageEvent::TextStart { partial } => {
                let mut base = partial.clone();
                let seed = match base.content.last() {
                    Some(ContentBlock::Text { text }) => {
                        let text = text.clone();
                        base.content.pop();
                        text
                    }
                    _ => String::new(),
                };
                self.base = Some(base);
                self.open = Some(OpenBlock::Text {
                    seed,
                    acc: String::new(),
                });
            }
            AssistantMessageEvent::ThinkingStart { partial } => {
                let mut base = partial.clone();
                let seed = match base.content.last() {
                    Some(ContentBlock::Thinking { text, .. }) => {
                        let text = text.clone();
                        base.content.pop();
                        text
                    }
                    _ => String::new(),
                };
                self.base = Some(base);
                self.open = Some(OpenBlock::Thinking {
                    seed,
                    acc: String::new(),
                });
            }
            AssistantMessageEvent::TextDelta { partial, delta } => match partial {
                Some(p) => {
                    self.base = Some(p.clone());
                    self.open = None;
                }
                None => match &mut self.open {
                    Some(OpenBlock::Text { acc, .. }) => acc.push_str(delta),
                    // Defensive: producer skipped TextStart.
                    _ => {
                        self.open = Some(OpenBlock::Text {
                            seed: String::new(),
                            acc: delta.clone(),
                        })
                    }
                },
            },
            AssistantMessageEvent::ThinkingDelta { partial, delta } => match partial {
                Some(p) => {
                    self.base = Some(p.clone());
                    self.open = None;
                }
                None => match &mut self.open {
                    Some(OpenBlock::Thinking { acc, .. }) => acc.push_str(delta),
                    _ => {
                        self.open = Some(OpenBlock::Thinking {
                            seed: String::new(),
                            acc: delta.clone(),
                        })
                    }
                },
            },
            // Raw args accumulate in args_acc; the call block itself is
            // already in the FunctioncallStart snapshot. A fat frame
            // still refreshes the snapshot (legacy behavior).
            AssistantMessageEvent::FunctioncallDelta {
                partial: Some(p), ..
            } => {
                self.base = Some(p.clone());
                self.open = None;
            }
            AssistantMessageEvent::FunctioncallDelta { partial: None, .. } => {}
            _ => {}
        }
    }

    /// The cumulative message: snapshot + the open block's accumulated text.
    fn current(&self) -> Option<AssistantMessage> {
        use crate::types::content::ContentBlock;
        let mut out = self.base.clone()?;
        match &self.open {
            Some(OpenBlock::Text { seed, acc }) if !(seed.is_empty() && acc.is_empty()) => {
                out.content.push(ContentBlock::Text {
                    text: format!("{seed}{acc}"),
                });
            }
            Some(OpenBlock::Thinking { seed, acc }) if !(seed.is_empty() && acc.is_empty()) => {
                out.content.push(ContentBlock::Thinking {
                    text: format!("{seed}{acc}"),
                    signature: None,
                });
            }
            _ => {}
        }
        Some(out)
    }
}

/// Fallback attribution for id-less delta frames (pre-id producers): blocks
/// stream in order, so guess the last function_call block of the partial.
fn open_call_id(partial: &AssistantMessage) -> Option<&str> {
    partial.content.iter().rev().find_map(|b| match b {
        crate::types::content::ContentBlock::FunctionCall { id, .. } => Some(id.as_str()),
        _ => None,
    })
}

/// Inject the in-flight raw-arguments tail into a coalesced partial as a
/// `_streaming` field beside the provider's placeholder, so UIs can render
/// the command being formed. Injected only while the accumulated text does
/// not yet parse — once it parses, the provider partial already carries the
/// real arguments. Returns None when nothing was injected.
fn enrich_streaming_args(
    partial: &AssistantMessage,
    acc: &std::collections::HashMap<String, String>,
) -> Option<AssistantMessage> {
    use crate::types::content::ContentBlock;
    if acc.is_empty() {
        return None;
    }
    let mut out: Option<AssistantMessage> = None;
    for (i, block) in partial.content.iter().enumerate() {
        let ContentBlock::FunctionCall { id, arguments, .. } = block else {
            continue;
        };
        let Some(raw) = acc.get(id) else { continue };
        if raw.is_empty() || serde_json::from_str::<Value>(raw).is_ok() {
            continue;
        }
        // Degraded placeholders are always objects; anything else is final.
        let Value::Object(map) = arguments else {
            continue;
        };
        let mut map = map.clone();
        map.insert(
            "_streaming".into(),
            Value::String(utf8_tail(raw, 1500).to_string()),
        );
        let msg = out.get_or_insert_with(|| partial.clone());
        if let ContentBlock::FunctionCall { arguments, .. } = &mut msg.content[i] {
            *arguments = Value::Object(map);
        }
    }
    out
}

fn utf8_tail(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

#[cfg(test)]
mod streaming_args_tests {
    use super::*;
    use crate::types::content::ContentBlock;
    use std::collections::HashMap;

    #[test]
    fn enrich_injects_tail_only_while_args_are_unparsed() {
        let mut m = empty_assistant("p", "m");
        m.content = vec![ContentBlock::FunctionCall {
            id: "c1".into(),
            function_id: "agent_trigger".into(),
            arguments: json!({ "function": "state::set" }),
        }];

        // No accumulated raw args → forwarded untouched.
        assert!(enrich_streaming_args(&m, &HashMap::new()).is_none());

        // Incomplete raw args → the live tail rides beside the salvaged
        // fields so the UI can show the command being formed.
        let mut acc = HashMap::new();
        acc.insert(
            "c1".to_string(),
            r#"{"function":"state::set","payload":{"value":"grow"#.to_string(),
        );
        let enriched = enrich_streaming_args(&m, &acc).unwrap();
        let ContentBlock::FunctionCall { arguments, .. } = &enriched.content[0] else {
            panic!("want function_call");
        };
        assert_eq!(arguments["function"], "state::set");
        assert!(arguments["_streaming"].as_str().unwrap().ends_with("grow"));

        // Complete raw args → the provider partial already carries them.
        acc.insert(
            "c1".to_string(),
            r#"{"function":"state::set","payload":{}}"#.to_string(),
        );
        assert!(enrich_streaming_args(&m, &acc).is_none());
    }

    #[test]
    fn open_call_id_is_the_last_call_block_and_tail_is_char_safe() {
        let mut m = empty_assistant("p", "m");
        m.content = vec![
            ContentBlock::FunctionCall {
                id: "c1".into(),
                function_id: "a".into(),
                arguments: json!({}),
            },
            ContentBlock::FunctionCall {
                id: "c2".into(),
                function_id: "b".into(),
                arguments: json!({}),
            },
        ];
        assert_eq!(open_call_id(&m), Some("c2"));
        // Tail never slices mid-codepoint.
        let s = format!("{}é", "x".repeat(1499));
        assert!(utf8_tail(&s, 1500).len() <= 1500);
        assert!(utf8_tail(&s, 1500).ends_with('é'));
    }

    #[test]
    fn functioncall_delta_frames_parse_with_and_without_id() {
        // Pre-id producers stream id-less deltas — they must still parse
        // (frames that fail to parse are silently dropped by the read loop).
        let partial = serde_json::to_value(empty_assistant("p", "m")).unwrap();
        let old = format!(r#"{{"type":"functioncall_delta","partial":{partial},"delta":"x"}}"#);
        match serde_json::from_str::<AssistantMessageEvent>(&old).unwrap() {
            AssistantMessageEvent::FunctioncallDelta { id, .. } => assert_eq!(id, ""),
            other => panic!("want functioncall_delta, got {other:?}"),
        }
        let new =
            format!(r#"{{"type":"functioncall_delta","partial":{partial},"delta":"x","id":"c1"}}"#);
        match serde_json::from_str::<AssistantMessageEvent>(&new).unwrap() {
            AssistantMessageEvent::FunctioncallDelta { id, .. } => assert_eq!(id, "c1"),
            other => panic!("want functioncall_delta, got {other:?}"),
        }
        // Slim frames (no partial at all) must parse too — they are the
        // post-contract-change producer shape.
        let slim = r#"{"type":"functioncall_delta","delta":"x","id":"c1"}"#;
        match serde_json::from_str::<AssistantMessageEvent>(slim).unwrap() {
            AssistantMessageEvent::FunctioncallDelta { partial, id, .. } => {
                assert!(partial.is_none());
                assert_eq!(id, "c1");
            }
            other => panic!("want functioncall_delta, got {other:?}"),
        }
        let slim_text = r#"{"type":"text_delta","delta":"hi"}"#;
        assert!(matches!(
            serde_json::from_str::<AssistantMessageEvent>(slim_text).unwrap(),
            AssistantMessageEvent::TextDelta { partial: None, .. }
        ));
    }

    #[test]
    fn tracker_accumulates_slim_text_deltas_from_the_boundary_snapshot() {
        use crate::types::content::ContentBlock;
        let mut tracker = PartialTracker::default();
        let base = empty_assistant("p", "m");
        tracker.apply(&AssistantMessageEvent::Start {
            partial: base.clone(),
        });
        tracker.apply(&AssistantMessageEvent::TextStart {
            partial: base.clone(),
        });
        tracker.apply(&AssistantMessageEvent::TextDelta {
            partial: None,
            delta: "Hel".into(),
        });
        tracker.apply(&AssistantMessageEvent::TextDelta {
            partial: None,
            delta: "lo".into(),
        });
        let cum = tracker.current().expect("cumulative message");
        assert_eq!(
            cum.content,
            vec![ContentBlock::Text {
                text: "Hello".into()
            }]
        );
        // The TextEnd snapshot is authoritative and replaces the open block.
        let mut done = base.clone();
        done.content = vec![ContentBlock::Text {
            text: "Hello!".into(),
        }];
        tracker.apply(&AssistantMessageEvent::TextEnd {
            partial: done.clone(),
        });
        assert_eq!(tracker.current().unwrap().content, done.content);
    }

    #[test]
    fn tracker_treats_fat_deltas_as_authoritative_snapshots() {
        use crate::types::content::ContentBlock;
        let mut tracker = PartialTracker::default();
        let mut fat = empty_assistant("p", "m");
        fat.content = vec![ContentBlock::Text {
            text: "cumulative".into(),
        }];
        tracker.apply(&AssistantMessageEvent::TextDelta {
            partial: Some(fat.clone()),
            delta: "e".into(),
        });
        // Legacy behavior: the fat partial IS the message; deltas are not
        // re-applied on top of it.
        assert_eq!(tracker.current().unwrap().content, fat.content);
    }

    #[test]
    fn tracker_keeps_thinking_and_call_blocks_across_boundaries() {
        use crate::types::content::ContentBlock;
        use serde_json::json;
        let mut tracker = PartialTracker::default();
        let base = empty_assistant("p", "m");
        tracker.apply(&AssistantMessageEvent::ThinkingStart {
            partial: base.clone(),
        });
        tracker.apply(&AssistantMessageEvent::ThinkingDelta {
            partial: None,
            delta: "hmm".into(),
        });
        assert_eq!(
            tracker.current().unwrap().content,
            vec![ContentBlock::Thinking {
                text: "hmm".into(),
                signature: None
            }]
        );
        // FunctioncallStart snapshot carries the call block; slim call-arg
        // deltas leave the tracker alone (args live in args_acc) so the
        // current view still shows the call block for open_call_id.
        let mut with_call = base.clone();
        with_call.content = vec![
            ContentBlock::Thinking {
                text: "hmm".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::FunctionCall {
                id: "c1".into(),
                function_id: "agent_trigger".into(),
                arguments: json!({}),
            },
        ];
        tracker.apply(&AssistantMessageEvent::FunctioncallStart {
            partial: with_call.clone(),
        });
        tracker.apply(&AssistantMessageEvent::FunctioncallDelta {
            partial: None,
            delta: r#"{"x":1"#.into(),
            id: "c1".into(),
        });
        let cum = tracker.current().unwrap();
        assert_eq!(cum.content, with_call.content);
        assert_eq!(open_call_id(&cum), Some("c1"));
    }
}
