//! `llm-router` adapters: model-limit resolution via
//! `router::models::budget`, the provider's prompt count via
//! `router::count_tokens`, and the summariser via `router::chat`.
//!
//! Both are soft dependencies — when the router is absent the resolver
//! errors (callers degrade to the conservative fallback) and the
//! summariser reports `Unavailable` (compaction maps it to
//! `status: "overflow"`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::IIIClient;
use iii_sdk::Pipe;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{CallRequest, ChannelItem};
use serde_json::{Value, json};

use crate::configuration::ConfigCell;
use crate::ports::{
    ModelBudget, ModelResolver, SummarizeError, SummarizeRequest, Summarizer, TurnKnobs,
};
use crate::types::{AgentFunction, AgentMessage, ErrorDetails, Failure, FailureKind, Model};

const MODELS_GET_TIMEOUT_MS: u64 = 5_000;
/// A local tokenizer answers in milliseconds; a provider's metering API is a
/// network call. Past this the assembly keeps its own estimate.
const COUNT_TOKENS_TIMEOUT_MS: u64 = 10_000;
/// Grace period for the stream reader to drain after `router::chat`
/// returns; the terminal `done`/`error` event lands before or with the
/// call response, so this only covers socket latency.
const READER_DRAIN_MS: u64 = 2_000;

fn is_unroutable(err: &Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("function_not_found") || msg.contains("not found") || msg.contains("no function")
}

/// `router::models::budget` — `{ model, effective_max_output_tokens } | null`.
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
    async fn get_model_budget(
        &self,
        provider: Option<&str>,
        id: &str,
    ) -> Result<Option<ModelBudget>, String> {
        let mut payload = json!({ "id": id });
        if let Some(p) = provider {
            payload["provider"] = json!(p);
        }
        let resp = match self
            .iii
            .call(CallRequest {
                function_id: "router::models::budget".to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(MODELS_GET_TIMEOUT_MS),
            })
            .await
        {
            Ok(resp) => resp,
            Err(err) if is_unroutable(&err) => {
                // Rolling upgrades may briefly run a context-manager newer than
                // the router. Preserve the former, conservative behavior until
                // `models::budget` is registered instead of dropping all the way
                // to the 8k fallback model.
                let response = self
                    .iii
                    .call(CallRequest {
                        function_id: "router::models::get".to_string(),
                        payload,
                        action: None,
                        timeout_ms: Some(MODELS_GET_TIMEOUT_MS),
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                if response.is_null() {
                    return Ok(None);
                }
                let model = response.get("model").cloned().ok_or_else(|| {
                    "router::models::get response has no `model` field".to_string()
                })?;
                if model.is_null() {
                    return Ok(None);
                }
                let model = serde_json::from_value::<Model>(model).map_err(|e| {
                    format!("router::models::get returned an unparseable model: {e}")
                })?;
                return Ok(Some(ModelBudget {
                    effective_max_output_tokens: model.max_output_tokens,
                    model,
                }));
            }
            Err(err) => return Err(err.to_string()),
        };

        if resp.is_null() {
            return Ok(None);
        }
        let model = resp
            .get("model")
            .cloned()
            .ok_or_else(|| "router::models::budget response has no `model` field".to_string())?;
        if model.is_null() {
            return Ok(None);
        }
        let effective_max_output_tokens = resp
            .get("effective_max_output_tokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                "router::models::budget response has no `effective_max_output_tokens` field"
                    .to_string()
            })?;
        serde_json::from_value::<Model>(model)
            .map(|model| {
                Some(ModelBudget {
                    model,
                    effective_max_output_tokens,
                })
            })
            .map_err(|e| format!("router::models::budget returned an unparseable model: {e}"))
    }

    async fn count_prompt_tokens(
        &self,
        provider: Option<&str>,
        id: &str,
        system_prompt: &str,
        tools: &[AgentFunction],
        messages: &[AgentMessage],
        knobs: TurnKnobs<'_>,
    ) -> Result<u64, String> {
        let payload = count_payload(provider, id, system_prompt, tools, messages, knobs);
        let resp = self
            .iii
            .call(CallRequest {
                function_id: "router::count_tokens".to_string(),
                payload,
                action: None,
                timeout_ms: Some(COUNT_TOKENS_TIMEOUT_MS),
            })
            .await
            .map_err(|e| e.to_string())?;
        resp.get("tokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| "router::count_tokens response has no `tokens` field".to_string())
    }
}

/// The `router::count_tokens` request: the turn's knobs ride only when set.
fn count_payload(
    provider: Option<&str>,
    id: &str,
    system_prompt: &str,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
    knobs: TurnKnobs<'_>,
) -> Value {
    let mut payload = json!({
        "model": id,
        "system_prompt": system_prompt,
        "tools": tools,
        "messages": messages,
    });
    if let Some(p) = provider {
        payload["provider"] = json!(p);
    }
    if let Some(level) = knobs.thinking_level {
        payload["thinking_level"] = json!(level);
    }
    if let Some(format) = knobs.response_format {
        payload["response_format"] = format.clone();
    }
    payload
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

// The bounded compaction pipeline can cancel an in-flight call. Dropping a
// JoinHandle alone detaches its reader; abort it when the owning call exits.
struct AbortReaderOnDrop(tokio::task::AbortHandle);

impl Drop for AbortReaderOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[async_trait]
impl Summarizer for RouterSummarizer {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError> {
        let Pipe { input, output } = self
            .iii
            .pipe()
            .await
            .map_err(|e| SummarizeError::Unavailable(format!("create_channel failed: {e}")))?;

        // Frames may arrive as text messages or as binary chunks; collect
        // both since the router's framing is provider-defined.
        let text_frames: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = text_frames.clone();
        let read_task = tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut binary = Vec::new();
            let mut frames = input.frames();
            while let Some(frame) = frames.next().await {
                match frame? {
                    ChannelItem::Text(text) => sink
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .push(text),
                    ChannelItem::Binary(bytes) => binary.extend_from_slice(&bytes),
                }
            }
            Ok::<Vec<u8>, Error>(binary)
        });
        let _reader_guard = AbortReaderOnDrop(read_task.abort_handle());

        let payload = chat_payload(&req, json!(output));

        // Read the outer budget from the live snapshot so a
        // summarizer_timeout_ms change takes effect without a restart.
        let timeout_ms = self.config.read().await.summarizer_timeout_ms;
        let call_result = self
            .iii
            .call(CallRequest {
                function_id: "router::chat".to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await;

        // The terminal event precedes the call response; give the
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

        let response = match call_result {
            Ok(v) => v,
            Err(e) => return Err(call_error(&e)),
        };
        if response.get("ok").and_then(Value::as_bool) == Some(false) {
            return Err(router_error(response.get("error")));
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

/// The router's error code for a request larger than the model's window.
const CONTEXT_OVERFLOW_CODE: &str = "router/context_overflow";

/// The `router::chat` request for one summariser turn: the caller's
/// `provider_options` (the conversation's account) ride only when set.
fn chat_payload(req: &SummarizeRequest, writer_ref: Value) -> Value {
    let mut payload = json!({
        "writer_ref": writer_ref,
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
    if let Some(options) = &req.provider_options {
        payload["provider_options"] = json!(options);
    }
    payload
}

/// A failed `router::chat` call (the bus refused or timed out) as a
/// summariser error.
fn call_error(error: &Error) -> SummarizeError {
    if is_unroutable(error) {
        return SummarizeError::Unavailable(error.to_string());
    }
    let detail = error.to_string();
    if detail.contains(CONTEXT_OVERFLOW_CODE) {
        return SummarizeError::ContextOverflow(detail);
    }
    let timed_out = matches!(error, Error::Timeout)
        || matches!(error, Error::Remote { code, .. } if code.to_lowercase().contains("timeout"));
    if timed_out {
        return SummarizeError::Failed(Failure::new(
            FailureKind::Timeout,
            "the summary took longer than its time budget",
        ));
    }
    SummarizeError::Failed(detail.into())
}

/// Router codes that mean no provider can serve the summary.
const UNAVAILABLE_CODES: [&str; 4] = [
    "router/provider_unavailable",
    "router/not_configured",
    "router/no_provider_for_model",
    "router/unknown_provider",
];

/// Router codes for a stream that ran out of time.
const TIMEOUT_CODES: [&str; 2] = ["router/stream_timeout", "router/stream_idle_timeout"];

/// A `router::chat` `{ ok: false, error }` reply as a summariser error: an
/// overflow stays distinguishable so the pipeline can retry a smaller
/// fragment; anything else keeps its cause (D162), the router's user-facing
/// `message` and its `error_details` (D147).
fn router_error(error: Option<&Value>) -> SummarizeError {
    let Some(error) = error else {
        return SummarizeError::Failed("unknown router error".into());
    };
    let code = error.get("code").and_then(Value::as_str).unwrap_or("");
    let kind = error.get("kind").and_then(Value::as_str);
    if code == CONTEXT_OVERFLOW_CODE || kind == Some("context_overflow") {
        return SummarizeError::ContextOverflow(error.to_string());
    }
    tracing::warn!(error = %error, "summariser: router::chat refused");
    let details = error
        .get("error_details")
        .and_then(ErrorDetails::from_value);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("the router refused the summary ({code})"));
    let failure_kind = if details.as_ref().is_some_and(|d| d.usage_limit) {
        FailureKind::UsageLimit
    } else if kind == Some("rate_limited") || code == "router/provider_rate_limited" {
        FailureKind::RateLimited
    } else if TIMEOUT_CODES.contains(&code) {
        FailureKind::Timeout
    } else if UNAVAILABLE_CODES.contains(&code) {
        FailureKind::Unavailable
    } else {
        FailureKind::Other
    };
    SummarizeError::Failed(Failure::new(failure_kind, message).with_details(details))
}

/// A terminal `error` frame's assistant message as a summariser error,
/// classified by its `error_kind` and `error_details` like a router reply.
fn stream_error(error: Option<&Value>) -> SummarizeError {
    let field = |name: &str| error.and_then(|e| e.get(name));
    let kind = field("error_kind").and_then(Value::as_str);
    let details = field("error_details").and_then(ErrorDetails::from_value);
    let text = field("error_message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| text_of_message(error));
    let message = format!("summariser stream error: {text}");
    if kind == Some("context_overflow") {
        return SummarizeError::ContextOverflow(message);
    }
    let failure_kind = if details.as_ref().is_some_and(|d| d.usage_limit) {
        FailureKind::UsageLimit
    } else if kind == Some("rate_limited") {
        FailureKind::RateLimited
    } else {
        FailureKind::Other
    };
    SummarizeError::Failed(Failure::new(failure_kind, message).with_details(details))
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
            Some("error") => return Err(stream_error(event.get("error"))),
            Some("stop") if event.get("stop_reason").and_then(Value::as_str) == Some("error") => {
                let detail = event
                    .get("error_message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown provider error");
                return Err(SummarizeError::Failed(
                    format!("summariser stream error: {detail}").into(),
                ));
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
        if block.get("type").and_then(Value::as_str) == Some("text")
            && let Some(text) = block.get("text").and_then(Value::as_str)
        {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_carries_the_turns_level_and_format_only_when_set() {
        let format = json!({ "type": "json" });
        let knobs = TurnKnobs {
            thinking_level: Some(crate::types::ThinkingLevel::Off),
            response_format: Some(&format),
        };
        let payload = count_payload(Some("deepseek"), "m", "", &[], &[], knobs);
        assert_eq!(payload["thinking_level"], "off");
        assert_eq!(payload["response_format"], format);
        assert_eq!(payload["provider"], "deepseek");

        let payload = count_payload(None, "m", "", &[], &[], TurnKnobs::default());
        assert!(payload.get("thinking_level").is_none(), "{payload}");
        assert!(payload.get("response_format").is_none(), "{payload}");
    }

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
    fn router_overflow_reply_is_a_context_overflow() {
        // The reply a 1M-token Claude window gave a 1.45M-token fragment.
        let error = json!({
            "code": "router/context_overflow",
            "kind": "context_overflow",
            "message": "This conversation is too large for model \"claude-opus-5-5\".",
            "retryable": false
        });
        assert!(matches!(
            router_error(Some(&error)),
            SummarizeError::ContextOverflow(detail) if detail.contains("too large")
        ));
        let other = json!({ "code": "router/rate_limited", "kind": "rate_limited" });
        assert!(matches!(
            router_error(Some(&other)),
            SummarizeError::Failed(_)
        ));
        assert!(matches!(router_error(None), SummarizeError::Failed(_)));
    }

    fn summarize_request(
        provider_options: Option<crate::types::ProviderOptions>,
    ) -> SummarizeRequest {
        SummarizeRequest {
            system_prompt: "system".into(),
            user_prompt: "history".into(),
            model: "claude-x".into(),
            provider: Some("claude-code".into()),
            provider_options,
        }
    }

    #[test]
    fn the_chat_request_carries_provider_options_only_when_set() {
        let options = crate::types::ProviderOptions::from([(
            "claude-code".to_string(),
            json!({ "account": "acct-work" }),
        )]);
        let payload = chat_payload(&summarize_request(Some(options)), json!("ref"));
        assert_eq!(
            payload["provider_options"],
            json!({ "claude-code": { "account": "acct-work" } })
        );
        assert_eq!(payload["provider"], "claude-code");
        assert_eq!(payload["writer_ref"], "ref");

        let payload = chat_payload(&summarize_request(None), json!("ref"));
        assert!(payload.get("provider_options").is_none(), "{payload}");
    }

    fn failure_of(error: SummarizeError) -> Failure {
        error.failure()
    }

    #[test]
    fn a_usage_limit_reply_keeps_its_details_and_message() {
        let error = json!({
            "code": "router/provider_rate_limited",
            "kind": "rate_limited",
            "message": "Provider \"claude-code\" reached the usage limit of its account. Try again after the limit resets.",
            "retryable": false,
            "detail": "429 from upstream",
            "error_details": { "usage_limit": true, "window": "five_hour", "resets_at_ms": 1_700_000_000_000u64 }
        });
        let failure = failure_of(router_error(Some(&error)));
        assert_eq!(failure.kind, FailureKind::UsageLimit);
        assert!(failure.message.contains("usage limit"), "{failure:?}");
        assert_eq!(
            failure.error_details,
            Some(ErrorDetails {
                usage_limit: true,
                window: Some(crate::types::UsageWindow::FiveHour),
                resets_at_ms: Some(1_700_000_000_000),
            })
        );
    }

    #[test]
    fn router_replies_map_to_their_cause() {
        let cases = [
            (
                json!({ "code": "router/provider_rate_limited", "kind": "rate_limited", "message": "busy" }),
                FailureKind::RateLimited,
            ),
            (
                // Details that do not say usage_limit leave a busy moment.
                json!({ "code": "router/provider_rate_limited", "kind": "rate_limited",
                        "message": "busy", "error_details": { "usage_limit": false } }),
                FailureKind::RateLimited,
            ),
            (
                json!({ "code": "router/stream_timeout", "kind": "transient", "message": "slow" }),
                FailureKind::Timeout,
            ),
            (
                json!({ "code": "router/stream_idle_timeout", "kind": "transient", "message": "idle" }),
                FailureKind::Timeout,
            ),
            (
                json!({ "code": "router/provider_unavailable", "kind": "transient", "message": "down" }),
                FailureKind::Unavailable,
            ),
            (
                json!({ "code": "router/no_provider_for_model", "kind": "permanent", "message": "none" }),
                FailureKind::Unavailable,
            ),
            (
                json!({ "code": "router/provider_auth_expired", "kind": "auth_expired", "message": "auth" }),
                FailureKind::Other,
            ),
            (
                json!({ "code": "router/context_overflow", "kind": "context_overflow", "message": "big" }),
                FailureKind::InputTooLarge,
            ),
        ];
        for (error, kind) in cases {
            let failure = failure_of(router_error(Some(&error)));
            assert_eq!(failure.kind, kind, "{error}");
        }
        assert_eq!(failure_of(router_error(None)).kind, FailureKind::Other);
        // A reply without a message still says which code refused it.
        let failure = failure_of(router_error(Some(&json!({ "code": "router/x" }))));
        assert!(failure.message.contains("router/x"), "{failure:?}");
    }

    #[test]
    fn call_errors_map_to_their_cause() {
        assert_eq!(
            failure_of(call_error(&Error::Timeout)).kind,
            FailureKind::Timeout
        );
        assert_eq!(
            failure_of(call_error(&Error::Remote {
                code: "TIMEOUT".into(),
                message: "invocation timed out".into(),
                stacktrace: None,
            }))
            .kind,
            FailureKind::Timeout
        );
        assert_eq!(
            failure_of(call_error(&Error::Remote {
                code: "function_not_found".into(),
                message: "router::chat".into(),
                stacktrace: None,
            }))
            .kind,
            FailureKind::Unavailable
        );
        assert!(matches!(
            call_error(&Error::Handler("router/context_overflow: too large".into())),
            SummarizeError::ContextOverflow(_)
        ));
        assert_eq!(
            failure_of(call_error(&Error::Handler("boom".into()))).kind,
            FailureKind::Other
        );
    }

    #[test]
    fn an_error_frame_keeps_its_usage_limit() {
        let frames = vec![frame(json!({ "type": "error", "error": {
            "role": "assistant", "content": [],
            "stop_reason": "error", "error_kind": "rate_limited",
            "error_message": "usage limit reached",
            "error_details": { "usage_limit": true, "window": "week", "unknown": 1 }
        }}))];
        let failure = failure_of(extract_summary(&frames).unwrap_err());
        assert_eq!(failure.kind, FailureKind::UsageLimit);
        assert!(
            failure.message.contains("usage limit reached"),
            "{failure:?}"
        );
        assert_eq!(
            failure.error_details.and_then(|d| d.window),
            Some(crate::types::UsageWindow::Week)
        );

        let frames = vec![frame(json!({ "type": "error", "error": {
            "error_kind": "context_overflow", "error_message": "too long"
        }}))];
        assert!(matches!(
            extract_summary(&frames).unwrap_err(),
            SummarizeError::ContextOverflow(_)
        ));
    }

    #[test]
    fn unreadable_error_details_never_cost_the_usage_limit() {
        let details = ErrorDetails::from_value(&json!({
            "usage_limit": true, "window": "fortnight", "resets_at_ms": "soon"
        }))
        .unwrap();
        assert!(details.usage_limit);
        assert_eq!(details.window, None);
        assert_eq!(details.resets_at_ms, None);
        assert_eq!(ErrorDetails::from_value(&json!("limit")), None);
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
