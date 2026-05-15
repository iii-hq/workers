//! Streaming client for the Anthropic Messages API.
//!
//! Implements the [`StreamFn`] contract used by the harness loop: never throws,
//! always returns an event-yielding stream that ends with `done` or `error`.
//!
//! Scope for 0.1.x: text and tool-use content blocks; no thinking blocks yet.
//! Cache control, transport selection, and OAuth refresh land alongside the
//! provider-base infrastructure in 0.2.

pub mod config;

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use harness_types::{
    AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind, StopReason, Usage,
};
use overflow_classify::classify_error;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[derive(Debug, Error)]
pub enum AnthropicError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// Anthropic-issued API key. Sent as `x-api-key`.
    ApiKey,
    /// OAuth access token. Sent as `Authorization: Bearer`.
    #[serde(rename = "oauth_bearer")]
    OAuthBearer,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AnthropicConfig {
    /// Header-bearer credential string. For [`AuthMode::ApiKey`] this is
    /// the raw API key sent as `x-api-key`. For [`AuthMode::OAuthBearer`]
    /// this is the OAuth access token sent as `Authorization: Bearer`.
    /// Always read `auth_mode` first to know which header convention to
    /// apply.
    pub credential_value: String,
    pub model: String,
    pub max_tokens: u32,
    pub api_url: String,
    pub auth_mode: AuthMode,
}

impl AnthropicConfig {
    /// Legacy builder kept for unit-test ergonomics. New code should call
    /// [`AnthropicConfig::with_credential`] which receives the resolved
    /// [`auth_credentials::Credential`] from `auth::get_token`.
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let key = std::env::var("ANTHROPIC_API_KEY")?;
        Ok(Self {
            credential_value: key,
            model: model.into(),
            max_tokens: 4096,
            api_url: config::DEFAULT_API_URL.into(),
            auth_mode: AuthMode::ApiKey,
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// `Credential::ApiKey` selects [`AuthMode::ApiKey`]; `Credential::OAuth`
    /// selects [`AuthMode::OAuthBearer`] and stashes the access token.
    pub fn with_credential(
        model: impl Into<String>,
        cred: &auth_credentials::Credential,
    ) -> anyhow::Result<Self> {
        let (key, auth_mode) = match cred {
            auth_credentials::Credential::ApiKey { key } => (key.clone(), AuthMode::ApiKey),
            auth_credentials::Credential::OAuth { access_token, .. } => {
                (access_token.clone(), AuthMode::OAuthBearer)
            }
        };
        Ok(Self {
            credential_value: key,
            model: model.into(),
            max_tokens: 4096,
            api_url: config::DEFAULT_API_URL.into(),
            auth_mode,
        })
    }

    pub fn with_max_tokens(mut self, max: u32) -> Self {
        self.max_tokens = max;
        self
    }

    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }
}

/// The HTTP auth header pair (name, value) for a given config. Pure
/// function; the request builder in `stream_inner` calls this and
/// applies the result to the outgoing `reqwest::RequestBuilder`.
pub fn auth_header_for(cfg: &AnthropicConfig) -> (&'static str, String) {
    match cfg.auth_mode {
        AuthMode::ApiKey => ("x-api-key", cfg.credential_value.clone()),
        AuthMode::OAuthBearer => ("authorization", format!("Bearer {}", cfg.credential_value)),
    }
}

// Wire request body is built dynamically via serde_json::json! to keep this file
// small; richer typed builders land alongside provider-base in 0.2.

/// Convert harness AgentMessages into Anthropic wire messages.
/// Skips Custom messages (filtered at convert_to_llm boundary).
///
/// Consecutive `FunctionResult` messages are merged into a single user wire
/// message containing one `tool_result` content block per result. Anthropic
/// rejects requests where parallel `tool_use` IDs from the previous assistant
/// turn are split across multiple user messages with the error
/// "tool_use ids were found without tool_result blocks immediately after".
pub fn to_wire_messages(messages: &[harness_types::AgentMessage]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut pending_results: Vec<serde_json::Value> = Vec::new();
    let flush_results = |pending: &mut Vec<serde_json::Value>, out: &mut Vec<serde_json::Value>| {
        if !pending.is_empty() {
            out.push(serde_json::json!({
                "role": "user",
                "content": std::mem::take(pending),
            }));
        }
    };
    for m in messages {
        match m {
            harness_types::AgentMessage::User(u) => {
                flush_results(&mut pending_results, &mut out);
                let content = u
                    .content
                    .iter()
                    .filter_map(content_block_to_wire)
                    .collect::<Vec<_>>();
                out.push(serde_json::json!({ "role": "user", "content": content }));
            }
            harness_types::AgentMessage::Assistant(a) => {
                flush_results(&mut pending_results, &mut out);
                let content = a
                    .content
                    .iter()
                    .filter_map(content_block_to_wire)
                    .collect::<Vec<_>>();
                out.push(serde_json::json!({ "role": "assistant", "content": content }));
            }
            harness_types::AgentMessage::FunctionResult(t) => {
                let text = t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text(tx) => Some(tx.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                pending_results.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": t.function_call_id,
                    "content": text,
                    "is_error": t.is_error,
                }));
            }
            harness_types::AgentMessage::Custom(_) => {}
        }
    }
    flush_results(&mut pending_results, &mut out);
    out
}

pub fn content_block_to_wire(b: &ContentBlock) -> Option<serde_json::Value> {
    match b {
        ContentBlock::Text(t) => Some(serde_json::json!({ "type": "text", "text": t.text })),
        ContentBlock::FunctionCall {
            id,
            function_id,
            arguments,
        } => Some(serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": encode_tool_name(function_id),
            "input": arguments,
        })),
        _ => None,
    }
}

/// Encode bus function ids (e.g. `shell::filesystem::ls`) into Anthropic's
/// tool-name regex `^[a-zA-Z0-9_-]{1,128}$`. Anthropic rejects `::`; encode
/// every `::` as `__` and decode on the way back. Tool names that already
/// contain `__` will round-trip incorrectly — current tools don't.
fn encode_tool_name(name: &str) -> String {
    name.replace("::", "__")
}

pub(crate) fn decode_tool_name(name: &str) -> String {
    name.replace("__", "::")
}

/// Tool definitions in Anthropic wire shape.
pub fn functions_to_wire(tools: &[harness_types::AgentFunction]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": encode_tool_name(&t.name),
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

// ─── Prompt caching ──────────────────────────────────────────────────────
//
// Anthropic's prompt cache reads stable prefixes at 10% input cost (and
// writes them at 125%). We mark up to 3 cacheable spans per request:
// the `system` block, the last entry of the `tools` array (which caches
// the whole tools array as a unit), and the last content block of the
// last "stable" assistant turn — i.e. one whose `tool_use` blocks all
// have matching downstream `tool_result` blocks (no in-flight tools).
// Anthropic rejects requests where the cacheable span hashes below the
// per-model minimum (~1024 tokens on most models), so each helper gates
// on a conservative character-length floor (~4 chars/token).
//
// Disabled by setting `HARNESS_ANTHROPIC_CACHE=0` in the environment.

const CACHE_MIN_CHARS: usize = 4096;
const CACHE_FLAG_ENV: &str = "HARNESS_ANTHROPIC_CACHE";

fn cache_enabled() -> bool {
    static CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| match std::env::var(CACHE_FLAG_ENV) {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "FALSE" | "False"),
        Err(_) => true,
    })
}

fn ephemeral_marker() -> serde_json::Value {
    serde_json::json!({ "type": "ephemeral" })
}

/// Build the `system` wire field. Emits the typed-block array form with a
/// `cache_control: ephemeral` marker when caching is enabled and the prompt
/// is long enough to be cache-eligible. Otherwise emits the plain-string
/// form (which Anthropic also accepts) so short system prompts don't trigger
/// HTTP 400 on too-small cacheable spans.
pub fn build_system_field(system_prompt: &str) -> serde_json::Value {
    if cache_enabled() && system_prompt.len() >= CACHE_MIN_CHARS {
        serde_json::json!([{
            "type": "text",
            "text": system_prompt,
            "cache_control": ephemeral_marker(),
        }])
    } else {
        serde_json::Value::String(system_prompt.to_string())
    }
}

/// Attach a `cache_control: ephemeral` marker to the last entry of the
/// `tools` array. Anthropic caches the entire prefix up to the marker, so
/// one marker on the last tool caches the whole tools array as a unit.
/// No-op when caching is disabled, the array is empty, or the serialized
/// size of the array falls below the cache-eligibility floor.
pub fn apply_tools_cache_control(tools: &mut [serde_json::Value]) {
    if !cache_enabled() || tools.is_empty() {
        return;
    }
    let serialized_size: usize = tools
        .iter()
        .map(|v| serde_json::to_string(v).map(|s| s.len()).unwrap_or(0))
        .sum();
    if serialized_size < CACHE_MIN_CHARS {
        return;
    }
    if let Some(last) = tools.last_mut() {
        if let Some(obj) = last.as_object_mut() {
            obj.insert("cache_control".into(), ephemeral_marker());
        }
    }
}

/// Stamp a `cache_control: ephemeral` marker on the last content block of
/// the most recent "stable" assistant turn — i.e. one whose `tool_use`
/// blocks all have matching downstream `tool_result` blocks. Marking an
/// unstable turn (in-flight tool calls) would cache a transient state and
/// invalidate on the next turn, defeating the point.
pub fn apply_messages_cache_anchor(wire: &mut [serde_json::Value]) {
    if !cache_enabled() || wire.is_empty() {
        return;
    }
    let last_stable = (0..wire.len())
        .rev()
        .find(|&idx| is_stable_assistant(wire, idx));
    let Some(idx) = last_stable else { return };
    let Some(content) = wire[idx].get_mut("content").and_then(|c| c.as_array_mut()) else {
        return;
    };
    if let Some(last_block) = content.last_mut() {
        if let Some(obj) = last_block.as_object_mut() {
            obj.insert("cache_control".into(), ephemeral_marker());
        }
    }
}

fn is_stable_assistant(wire: &[serde_json::Value], idx: usize) -> bool {
    let msg = &wire[idx];
    if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return false;
    }
    let tool_use_ids: Vec<&str> = msg
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        b.get("id").and_then(|i| i.as_str())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if tool_use_ids.is_empty() {
        return true;
    }
    tool_use_ids
        .iter()
        .all(|id| has_downstream_tool_result(&wire[idx + 1..], id))
}

fn has_downstream_tool_result(later: &[serde_json::Value], id: &str) -> bool {
    later.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_array())
                .map(|blocks| {
                    blocks.iter().any(|b| {
                        b.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                            && b.get("tool_use_id").and_then(|i| i.as_str()) == Some(id)
                    })
                })
                .unwrap_or(false)
    })
}

/// Stream a response from Anthropic. Returns an event stream that closes with
/// `done` on success or `error` on failure. Never throws.
pub async fn stream(
    cfg: Arc<AnthropicConfig>,
    system_prompt: String,
    messages: Vec<harness_types::AgentMessage>,
    tools: Vec<harness_types::AgentFunction>,
) -> ReceiverStream<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    // tokio::spawn drops the caller's OTel context; capture it here so
    // the HTTP span inside stream_inner is parented to the invocation
    // span (and inherits iii.session.id baggage for "Group by session").
    let otel_cx = iii_sdk::capture_otel_context();
    tokio::spawn(async move {
        let result = otel_cx
            .attach(stream_inner(
                cfg,
                system_prompt,
                messages,
                tools,
                tx.clone(),
            ))
            .await;
        if let Err(e) = result {
            // Encode any error as final error event per the no-throw contract.
            let final_msg = AssistantMessage {
                content: vec![ContentBlock::Text(harness_types::TextContent {
                    text: e.to_string(),
                })],
                stop_reason: StopReason::Error,
                error_message: Some(e.to_string()),
                error_kind: Some(classify_error(&e.to_string(), None)),
                usage: None,
                model: "anthropic".into(),
                provider: "anthropic".into(),
                timestamp: chrono::Utc::now().timestamp_millis(),
            };
            let _ = tx
                .send(AssistantMessageEvent::Error { error: final_msg })
                .await;
        }
    });
    ReceiverStream::new(rx)
}

#[derive(Debug, Default)]
struct PartialState {
    text_blocks: Vec<String>,
    function_calls: Vec<PartialFunctionCall>,
    usage: Usage,
    stop_reason: Option<StopReason>,
    error_message: Option<String>,
}

#[derive(Debug, Default)]
struct PartialFunctionCall {
    id: String,
    function_id: String,
    args_json: String,
}

async fn stream_inner(
    cfg: Arc<AnthropicConfig>,
    system_prompt: String,
    messages: Vec<harness_types::AgentMessage>,
    tools: Vec<harness_types::AgentFunction>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), AnthropicError> {
    // Pre-HTTP marshal: wire-message conversion + serde_json::to_value
    // + reqwest client build + header assembly. Was the ~60ms gap
    // between `call provider::anthropic::complete` start and the POST
    // span start in the trace.
    let (client, request) = iii_sdk::run_in_span(
        "anthropic.request.build",
        Some(iii_sdk::SpanKind::Internal),
        || async {
            let mut wire_messages = to_wire_messages(&messages);
            apply_messages_cache_anchor(&mut wire_messages);
            let mut wire_tools = functions_to_wire(&tools);
            apply_tools_cache_control(&mut wire_tools);
            let body = serde_json::json!({
                "model": cfg.model,
                "max_tokens": cfg.max_tokens,
                "system": build_system_field(&system_prompt),
                "messages": wire_messages,
                "tools": wire_tools,
                "stream": true,
            });
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_mins(2))
                .build()?;
            let (header_name, header_value) = auth_header_for(&cfg);
            let request = client
                .post(&cfg.api_url)
                .header(header_name, header_value)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .build()?;
            Ok::<_, AnthropicError>((client, request))
        },
    )
    .await?;
    let resp = iii_sdk::execute_traced_request(&client, request).await?;

    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        let kind = classify_error(&err_text, Some(status.as_u16()));
        let final_msg = AssistantMessage {
            content: vec![ContentBlock::Text(harness_types::TextContent {
                text: err_text.clone(),
            })],
            stop_reason: StopReason::Error,
            error_message: Some(err_text),
            error_kind: Some(kind),
            usage: None,
            model: cfg.model.clone(),
            provider: "anthropic".into(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };
        let _ = tx
            .send(AssistantMessageEvent::Error { error: final_msg })
            .await;
        return Ok(());
    }

    let partial_msg = AssistantMessage {
        content: Vec::new(),
        stop_reason: StopReason::End,
        error_message: None,
        error_kind: None,
        usage: None,
        model: cfg.model.clone(),
        provider: "anthropic".into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    let _ = tx
        .send(AssistantMessageEvent::Start {
            partial: partial_msg.clone(),
        })
        .await;

    let mut state = PartialState {
        stop_reason: Some(StopReason::End),
        ..Default::default()
    };

    // SSE consume: chunked event-stream parsing + delta forwarding.
    // Was the ~260ms gap between POST end and complete-span end.
    iii_sdk::run_in_span(
        "anthropic.stream.consume",
        Some(iii_sdk::SpanKind::Internal),
        || async {
            let mut bytes_stream = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = bytes_stream.next().await {
                let chunk: Bytes = chunk?;
                let text = String::from_utf8_lossy(&chunk);
                buf.push_str(&text);

                while let Some(idx) = buf.find("\n\n") {
                    let event = buf[..idx].to_string();
                    buf.drain(..=idx + 1);
                    handle_sse_event(&event, &mut state, &tx, &cfg.model).await;
                }
            }
            Ok::<_, AnthropicError>(())
        },
    )
    .await?;

    let final_message = build_final(&state, &cfg.model);
    let _ = tx
        .send(AssistantMessageEvent::Done {
            message: final_message,
        })
        .await;
    Ok(())
}

async fn handle_sse_event(
    event_block: &str,
    state: &mut PartialState,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    model: &str,
) {
    let mut data: Option<&str> = None;
    for line in event_block.lines() {
        if let Some(d) = line.strip_prefix("data: ") {
            data = Some(d);
        }
    }
    let Some(data) = data else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };
    let Some(event_type) = parsed.get("type").and_then(|v| v.as_str()) else {
        return;
    };

    match event_type {
        "content_block_start" => {
            let block = parsed.get("content_block");
            let block_type = block.and_then(|b| b.get("type")).and_then(|v| v.as_str());
            match block_type {
                Some("text") => {
                    state.text_blocks.push(String::new());
                    let _ = tx
                        .send(AssistantMessageEvent::TextStart {
                            partial: build_partial(state, model),
                        })
                        .await;
                }
                Some("tool_use") => {
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let function_id = block
                        .and_then(|b| b.get("name"))
                        .and_then(|v| v.as_str())
                        .map(decode_tool_name)
                        .unwrap_or_default();
                    state.function_calls.push(PartialFunctionCall {
                        id,
                        function_id,
                        args_json: String::new(),
                    });
                    let _ = tx
                        .send(AssistantMessageEvent::FunctioncallStart {
                            partial: build_partial(state, model),
                        })
                        .await;
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let delta = parsed.get("delta");
            let delta_type = delta.and_then(|d| d.get("type")).and_then(|v| v.as_str());
            match delta_type {
                Some("text_delta") => {
                    let text = delta
                        .and_then(|d| d.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(last) = state.text_blocks.last_mut() {
                        last.push_str(&text);
                    }
                    let _ = tx
                        .send(AssistantMessageEvent::TextDelta {
                            partial: build_partial(state, model),
                            delta: text,
                        })
                        .await;
                }
                Some("input_json_delta") => {
                    let json = delta
                        .and_then(|d| d.get("partial_json"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(last) = state.function_calls.last_mut() {
                        last.args_json.push_str(&json);
                    }
                    let _ = tx
                        .send(AssistantMessageEvent::FunctioncallDelta {
                            partial: build_partial(state, model),
                            delta: json,
                        })
                        .await;
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            // Either text or tool — emit the right end event using the most recent block.
            if !state.function_calls.is_empty()
                && state.text_blocks.last().is_none_or(String::is_empty)
            {
                // tool call just stopped (heuristic; Anthropic guarantees ordering)
            }
            // Generic end events at this stage are best-effort:
            let _ = tx
                .send(AssistantMessageEvent::TextEnd {
                    partial: build_partial(state, model),
                })
                .await;
        }
        "message_delta" => {
            if let Some(stop) = parsed
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
            {
                state.stop_reason = Some(map_stop_reason(stop));
            }
            if let Some(usage) = parsed.get("usage") {
                merge_usage(usage, &mut state.usage);
            }
        }
        "message_stop" => {
            let _ = tx
                .send(AssistantMessageEvent::Stop {
                    stop_reason: state.stop_reason.unwrap_or(StopReason::End),
                    error_message: state.error_message.clone(),
                    error_kind: None,
                })
                .await;
        }
        "message_start" => {
            if let Some(usage) = parsed.get("message").and_then(|m| m.get("usage")) {
                merge_usage(usage, &mut state.usage);
            }
        }
        _ => {}
    }
}

fn merge_usage(usage: &serde_json::Value, into: &mut Usage) {
    if let Some(v) = usage
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.input += v;
    }
    if let Some(v) = usage
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.output += v;
    }
    if let Some(v) = usage
        .get("cache_read_input_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.cache_read += v;
    }
    if let Some(v) = usage
        .get("cache_creation_input_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.cache_write += v;
    }
}

fn map_stop_reason(s: &str) -> StopReason {
    match s {
        "end_turn" => StopReason::End,
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::FunctionCall,
        "stop_sequence" => StopReason::End,
        _ => StopReason::End,
    }
}

fn build_partial(state: &PartialState, model: &str) -> AssistantMessage {
    AssistantMessage {
        content: build_content(state),
        stop_reason: state.stop_reason.unwrap_or(StopReason::End),
        error_message: state.error_message.clone(),
        error_kind: None,
        usage: Some(state.usage),
        model: model.to_string(),
        provider: "anthropic".into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

fn build_final(state: &PartialState, model: &str) -> AssistantMessage {
    let mut msg = build_partial(state, model);
    msg.stop_reason = state.stop_reason.unwrap_or(StopReason::End);
    msg
}

pub(crate) fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    let mut content = Vec::new();
    for t in &state.text_blocks {
        if !t.is_empty() {
            content.push(ContentBlock::Text(harness_types::TextContent {
                text: t.clone(),
            }));
        }
    }
    for tc in &state.function_calls {
        let args = if tc.args_json.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str::<serde_json::Value>(&tc.args_json)
                .unwrap_or(serde_json::Value::Null)
        };
        content.push(ContentBlock::FunctionCall {
            id: tc.id.clone(),
            function_id: tc.function_id.clone(),
            arguments: args,
        });
    }
    content
}

/// Register `provider::anthropic::complete` on the iii bus.
///
/// The handler decodes `{ config, system_prompt, messages, tools }`, calls
/// [`stream`], drains the resulting event stream, and returns
/// `{ events: [<AssistantMessageEvent>...] }`.
pub async fn register_with_iii(
    iii: &iii_sdk::III,
    worker_cfg: &config::WorkerConfig,
) -> anyhow::Result<()> {
    let default_max = worker_cfg.default_max_tokens;
    let default_url = worker_cfg.default_api_url.clone();
    provider_base::register_provider_complete::<AnthropicConfig, _, _, _, _>(
        iii,
        "anthropic",
        move |model: &str, cred: &auth_credentials::Credential| {
            AnthropicConfig::with_credential(model, cred).map(|c| {
                c.with_max_tokens(default_max)
                    .with_api_url(default_url.clone())
            })
        },
        stream,
    );
    Ok(())
}

/// Convenience: collect a stream into a final AssistantMessage.
pub async fn collect(mut stream: ReceiverStream<AssistantMessageEvent>) -> AssistantMessage {
    let mut last: Option<AssistantMessage> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            AssistantMessageEvent::Done { message } => return message,
            AssistantMessageEvent::Error { error } => return error,
            AssistantMessageEvent::Start { partial }
            | AssistantMessageEvent::TextStart { partial }
            | AssistantMessageEvent::TextDelta { partial, .. }
            | AssistantMessageEvent::TextEnd { partial }
            | AssistantMessageEvent::FunctioncallStart { partial }
            | AssistantMessageEvent::FunctioncallDelta { partial, .. }
            | AssistantMessageEvent::FunctioncallEnd { partial }
            | AssistantMessageEvent::ThinkingStart { partial }
            | AssistantMessageEvent::ThinkingDelta { partial, .. }
            | AssistantMessageEvent::ThinkingEnd { partial } => {
                last = Some(partial);
            }
            _ => {}
        }
    }
    last.unwrap_or_else(|| AssistantMessage {
        content: Vec::new(),
        stop_reason: StopReason::Error,
        error_message: Some("stream closed without final".into()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: "anthropic".into(),
        provider: "anthropic".into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{AgentMessage, ContentBlock, TextContent, UserMessage};

    #[test]
    fn user_message_converts_to_wire() {
        let msgs = vec![AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: "hello".into(),
            })],
            timestamp: 1,
        })];
        let wire = to_wire_messages(&msgs);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[0]["content"][0]["type"], "text");
        assert_eq!(wire[0]["content"][0]["text"], "hello");
    }

    #[test]
    fn tool_result_converts_to_user_with_tool_result_block() {
        let msgs = vec![AgentMessage::FunctionResult(
            harness_types::FunctionResultMessage {
                function_call_id: "tc1".into(),
                function_id: "read".into(),
                content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
                details: serde_json::json!({}),
                is_error: false,
                timestamp: 2,
            },
        )];
        let wire = to_wire_messages(&msgs);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[0]["content"][0]["type"], "tool_result");
        assert_eq!(wire[0]["content"][0]["tool_use_id"], "tc1");
    }

    /// Anthropic requires that all tool_result blocks for an assistant's
    /// parallel tool_use blocks live in a single immediately-following user
    /// message. Consecutive FunctionResult AgentMessages must collapse into
    /// one wire user message. Otherwise the API rejects the request with:
    ///   "tool_use ids were found without tool_result blocks immediately after"
    #[test]
    fn parallel_function_results_collapse_into_one_user_message() {
        let mk = |id: &str| {
            AgentMessage::FunctionResult(harness_types::FunctionResultMessage {
                function_call_id: id.into(),
                function_id: "read".into(),
                content: vec![ContentBlock::Text(TextContent { text: id.into() })],
                details: serde_json::json!({}),
                is_error: false,
                timestamp: 0,
            })
        };
        let msgs = vec![mk("a"), mk("b"), mk("c")];
        let wire = to_wire_messages(&msgs);
        assert_eq!(
            wire.len(),
            1,
            "three FunctionResults must produce one user message"
        );
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[0]["content"].as_array().unwrap().len(), 3);
        assert_eq!(wire[0]["content"][0]["tool_use_id"], "a");
        assert_eq!(wire[0]["content"][1]["tool_use_id"], "b");
        assert_eq!(wire[0]["content"][2]["tool_use_id"], "c");
    }

    #[test]
    fn map_stop_reason_known_values() {
        assert!(matches!(map_stop_reason("end_turn"), StopReason::End));
        assert!(matches!(map_stop_reason("max_tokens"), StopReason::Length));
        assert!(matches!(
            map_stop_reason("tool_use"),
            StopReason::FunctionCall
        ));
    }

    #[test]
    fn merge_usage_accumulates() {
        let mut u = Usage::default();
        merge_usage(
            &serde_json::json!({"input_tokens": 10, "output_tokens": 20}),
            &mut u,
        );
        merge_usage(
            &serde_json::json!({"input_tokens": 5, "output_tokens": 6}),
            &mut u,
        );
        assert_eq!(u.input, 15);
        assert_eq!(u.output, 26);
    }

    #[test]
    fn merge_usage_captures_cache_fields() {
        let mut u = Usage::default();
        merge_usage(
            &serde_json::json!({
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 80,
                "cache_creation_input_tokens": 20,
            }),
            &mut u,
        );
        assert_eq!(u.cache_read, 80);
        assert_eq!(u.cache_write, 20);
    }

    #[test]
    fn build_system_field_short_returns_string() {
        let out = build_system_field("hi");
        assert!(out.is_string());
        assert_eq!(out.as_str(), Some("hi"));
    }

    #[test]
    fn build_system_field_long_returns_typed_block_with_cache_marker() {
        let long = "x".repeat(CACHE_MIN_CHARS);
        let out = build_system_field(&long);
        let arr = out.as_array().expect("typed-block array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"].as_str().unwrap().len(), CACHE_MIN_CHARS);
        assert_eq!(arr[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn apply_tools_cache_control_skips_empty() {
        let mut tools: Vec<serde_json::Value> = vec![];
        apply_tools_cache_control(&mut tools);
        assert!(tools.is_empty());
    }

    #[test]
    fn apply_tools_cache_control_skips_small_arrays() {
        // A single tiny tool entry — far below the 4 KB floor.
        let mut tools = vec![serde_json::json!({
            "name": "agent_call",
            "description": "noop",
            "input_schema": {"type": "object"},
        })];
        apply_tools_cache_control(&mut tools);
        assert!(
            tools[0].get("cache_control").is_none(),
            "tiny tools array must not be marked (would 400 on Anthropic)"
        );
    }

    #[test]
    fn apply_tools_cache_control_marks_last_when_eligible() {
        // Pad description so the serialized array exceeds CACHE_MIN_CHARS.
        let bulky = "x".repeat(CACHE_MIN_CHARS);
        let mut tools = vec![
            serde_json::json!({"name": "a", "description": "small", "input_schema": {}}),
            serde_json::json!({"name": "b", "description": bulky, "input_schema": {}}),
        ];
        apply_tools_cache_control(&mut tools);
        assert!(
            tools[0].get("cache_control").is_none(),
            "only the last entry is marked"
        );
        assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn apply_messages_cache_anchor_marks_last_stable_assistant() {
        let mut wire = vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "first reply"}]
            }),
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "more"}]}),
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "second reply"}]
            }),
        ];
        apply_messages_cache_anchor(&mut wire);
        assert!(
            wire[1]["content"][0].get("cache_control").is_none(),
            "earlier assistant should not be marked"
        );
        assert_eq!(wire[3]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn apply_messages_cache_anchor_skips_assistant_with_unresolved_tool_use() {
        let mut wire = vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
            serde_json::json!({
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "running"},
                    {"type": "tool_use", "id": "tc1", "name": "shell__run", "input": {}}
                ]
            }),
            // No matching tool_result follows — assistant is "in-flight".
        ];
        apply_messages_cache_anchor(&mut wire);
        for block in wire[1]["content"].as_array().unwrap() {
            assert!(
                block.get("cache_control").is_none(),
                "in-flight assistant must not be marked"
            );
        }
    }

    #[test]
    fn apply_messages_cache_anchor_marks_assistant_when_tool_result_follows() {
        let mut wire = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "running"},
                    {"type": "tool_use", "id": "tc1", "name": "shell__run", "input": {}}
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "tc1", "content": "ok"}]
            }),
        ];
        apply_messages_cache_anchor(&mut wire);
        // The tool_use is the last content block of the assistant; the
        // marker lands there.
        let tool_use = &wire[0]["content"][1];
        assert_eq!(tool_use["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn apply_messages_cache_anchor_noop_on_empty() {
        let mut wire: Vec<serde_json::Value> = vec![];
        apply_messages_cache_anchor(&mut wire);
        assert!(wire.is_empty());
    }

    #[test]
    fn with_credential_api_key() {
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-ant-foo".into(),
        };
        let cfg = AnthropicConfig::with_credential("claude-sonnet-4-6", &cred).unwrap();
        assert_eq!(cfg.credential_value, "sk-ant-foo");
        assert_eq!(cfg.model, "claude-sonnet-4-6");
        assert!(matches!(cfg.auth_mode, AuthMode::ApiKey));
    }

    #[test]
    fn with_credential_oauth_picks_bearer_mode() {
        let cred = auth_credentials::Credential::OAuth {
            access_token: "tok-bar".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            provider_extra: serde_json::Value::Null,
        };
        let cfg = AnthropicConfig::with_credential("claude-sonnet-4-6", &cred).unwrap();
        assert_eq!(cfg.credential_value, "tok-bar");
        assert!(matches!(cfg.auth_mode, AuthMode::OAuthBearer));
    }

    #[test]
    fn auth_mode_serialises_with_explicit_oauth_bearer() {
        let s = serde_json::to_string(&AuthMode::OAuthBearer).unwrap();
        assert_eq!(s, "\"oauth_bearer\"");
        let s = serde_json::to_string(&AuthMode::ApiKey).unwrap();
        assert_eq!(s, "\"api_key\"");

        // Round-trip.
        let parsed: AuthMode = serde_json::from_str("\"oauth_bearer\"").unwrap();
        assert!(matches!(parsed, AuthMode::OAuthBearer));
    }

    #[test]
    fn auth_header_for_api_key_uses_x_api_key() {
        let cfg = AnthropicConfig {
            credential_value: "sk-ant-xyz".into(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            api_url: "https://api.anthropic.com/v1/messages".into(),
            auth_mode: AuthMode::ApiKey,
        };
        let (name, value) = auth_header_for(&cfg);
        assert_eq!(name, "x-api-key");
        assert_eq!(value, "sk-ant-xyz");
    }

    #[test]
    fn build_content_mixed_text_and_tool_partial_state() {
        let state = PartialState {
            text_blocks: vec!["hello".into(), String::new(), "world".into()],
            function_calls: vec![PartialFunctionCall {
                id: "tc1".into(),
                function_id: "read".into(),
                args_json: "{\"path\":\"/tmp/x\"}".into(),
            }],
            usage: Usage::default(),
            stop_reason: Some(StopReason::End),
            error_message: None,
        };
        let content = build_content(&state);
        // Empty text block is skipped; two text + one tool call remain.
        assert_eq!(content.len(), 3);
        match &content[0] {
            ContentBlock::Text(t) => assert_eq!(t.text, "hello"),
            other => panic!("expected text, got {other:?}"),
        }
        match &content[1] {
            ContentBlock::Text(t) => assert_eq!(t.text, "world"),
            other => panic!("expected text, got {other:?}"),
        }
        match &content[2] {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => {
                assert_eq!(id, "tc1");
                assert_eq!(function_id, "read");
                assert_eq!(arguments["path"], "/tmp/x");
            }
            other => panic!("expected tool call, got {other:?}"),
        }
    }

    #[test]
    fn build_content_invalid_args_json_falls_back_to_null() {
        let state = PartialState {
            text_blocks: vec![],
            function_calls: vec![PartialFunctionCall {
                id: "tc1".into(),
                function_id: "read".into(),
                args_json: "not-json".into(),
            }],
            usage: Usage::default(),
            stop_reason: None,
            error_message: None,
        };
        let content = build_content(&state);
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::FunctionCall { arguments, .. } => {
                assert!(arguments.is_null());
            }
            other => panic!("expected tool call, got {other:?}"),
        }
    }

    #[test]
    fn auth_header_for_oauth_uses_bearer() {
        let cfg = AnthropicConfig {
            credential_value: "tok-abc".into(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            api_url: "https://api.anthropic.com/v1/messages".into(),
            auth_mode: AuthMode::OAuthBearer,
        };
        let (name, value) = auth_header_for(&cfg);
        assert_eq!(name, "authorization");
        assert_eq!(value, "Bearer tok-abc");
    }
}
