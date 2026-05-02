//! Streaming client for the Anthropic Messages API.
//!
//! Implements the [`StreamFn`] contract used by the harness loop: never throws,
//! always returns an event-yielding stream that ends with `done` or `error`.
//!
//! Scope for 0.1.x: text and tool-use content blocks; no thinking blocks yet.
//! Cache control, transport selection, and OAuth refresh land alongside the
//! provider-base infrastructure in 0.2.

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
            api_url: "https://api.anthropic.com/v1/messages".into(),
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
            api_url: "https://api.anthropic.com/v1/messages".into(),
            auth_mode,
        })
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
pub fn to_wire_messages(messages: &[harness_types::AgentMessage]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for m in messages {
        match m {
            harness_types::AgentMessage::User(u) => {
                let content = u
                    .content
                    .iter()
                    .filter_map(content_block_to_wire)
                    .collect::<Vec<_>>();
                out.push(serde_json::json!({ "role": "user", "content": content }));
            }
            harness_types::AgentMessage::Assistant(a) => {
                let content = a
                    .content
                    .iter()
                    .filter_map(content_block_to_wire)
                    .collect::<Vec<_>>();
                out.push(serde_json::json!({ "role": "assistant", "content": content }));
            }
            harness_types::AgentMessage::ToolResult(t) => {
                let text = t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text(tx) => Some(tx.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": t.tool_call_id,
                        "content": text,
                        "is_error": t.is_error,
                    }],
                }));
            }
            harness_types::AgentMessage::Custom(_) => {}
        }
    }
    out
}

fn content_block_to_wire(b: &ContentBlock) -> Option<serde_json::Value> {
    match b {
        ContentBlock::Text(t) => Some(serde_json::json!({ "type": "text", "text": t.text })),
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
        } => Some(serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": arguments,
        })),
        _ => None,
    }
}

/// Tool definitions in Anthropic wire shape.
pub fn tools_to_wire(tools: &[harness_types::AgentTool]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

/// Stream a response from Anthropic. Returns an event stream that closes with
/// `done` on success or `error` on failure. Never throws.
pub async fn stream(
    cfg: Arc<AnthropicConfig>,
    system_prompt: String,
    messages: Vec<harness_types::AgentMessage>,
    tools: Vec<harness_types::AgentTool>,
) -> ReceiverStream<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        if let Err(e) = stream_inner(cfg, system_prompt, messages, tools, tx.clone()).await {
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
    tool_calls: Vec<PartialToolCall>,
    usage: Usage,
    stop_reason: Option<StopReason>,
    error_message: Option<String>,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args_json: String,
}

async fn stream_inner(
    cfg: Arc<AnthropicConfig>,
    system_prompt: String,
    messages: Vec<harness_types::AgentMessage>,
    tools: Vec<harness_types::AgentTool>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), AnthropicError> {
    let body = serde_json::json!({
        "model": cfg.model,
        "max_tokens": cfg.max_tokens,
        "system": system_prompt,
        "messages": to_wire_messages(&messages),
        "tools": tools_to_wire(&tools),
        "stream": true,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let (header_name, header_value) = auth_header_for(&cfg);
    let resp = client
        .post(&cfg.api_url)
        .header(header_name, header_value)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

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
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    state.tool_calls.push(PartialToolCall {
                        id,
                        name,
                        args_json: String::new(),
                    });
                    let _ = tx
                        .send(AssistantMessageEvent::ToolcallStart {
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
                    if let Some(last) = state.tool_calls.last_mut() {
                        last.args_json.push_str(&json);
                    }
                    let _ = tx
                        .send(AssistantMessageEvent::ToolcallDelta {
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
            if !state.tool_calls.is_empty() && state.text_blocks.last().is_none_or(String::is_empty)
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
        "tool_use" => StopReason::Tool,
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

fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    let mut content = Vec::new();
    for t in &state.text_blocks {
        if !t.is_empty() {
            content.push(ContentBlock::Text(harness_types::TextContent {
                text: t.clone(),
            }));
        }
    }
    for tc in &state.tool_calls {
        let args = if tc.args_json.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str::<serde_json::Value>(&tc.args_json)
                .unwrap_or(serde_json::Value::Null)
        };
        content.push(ContentBlock::ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: args,
        });
    }
    content
}

/// Register `provider::anthropic::stream` on the iii bus.
///
/// The handler decodes `{ config, system_prompt, messages, tools }`, calls
/// [`stream`], drains the resulting event stream, and returns
/// `{ events: [<AssistantMessageEvent>...] }`.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<AnthropicConfig, _, _, _, _>(
        iii,
        "anthropic",
        |model: &str, cred: &auth_credentials::Credential| {
            AnthropicConfig::with_credential(model, cred)
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
            | AssistantMessageEvent::ToolcallStart { partial }
            | AssistantMessageEvent::ToolcallDelta { partial, .. }
            | AssistantMessageEvent::ToolcallEnd { partial }
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
        let msgs = vec![AgentMessage::ToolResult(harness_types::ToolResultMessage {
            tool_call_id: "tc1".into(),
            tool_name: "read".into(),
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            details: serde_json::json!({}),
            is_error: false,
            timestamp: 2,
        })];
        let wire = to_wire_messages(&msgs);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[0]["content"][0]["type"], "tool_result");
        assert_eq!(wire[0]["content"][0]["tool_use_id"], "tc1");
    }

    #[test]
    fn map_stop_reason_known_values() {
        assert!(matches!(map_stop_reason("end_turn"), StopReason::End));
        assert!(matches!(map_stop_reason("max_tokens"), StopReason::Length));
        assert!(matches!(map_stop_reason("tool_use"), StopReason::Tool));
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
