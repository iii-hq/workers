//! Generic OpenAI Chat Completions streaming client.
//!
//! Reused by every provider that speaks the OpenAI completions wire shape:
//! Groq, Cerebras, xAI, OpenRouter, DeepSeek, Mistral, Fireworks, Kimi for
//! Coding, MiniMax, z.ai, HuggingFace, Vercel AI Gateway, OpenCode Zen,
//! OpenCode Go. Per-provider crates are thin config wrappers over this fn.

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use harness_types::{
    AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason,
    TextContent, ToolCall, Usage,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::errors::{classify_provider_error, error_event};
use crate::sse::parse_sse_block;

/// Configuration for a single Chat Completions streaming call.
#[derive(Debug, Clone)]
pub struct ChatCompletionsConfig {
    /// Full URL, e.g. `https://api.groq.com/openai/v1/chat/completions`.
    pub url: String,
    /// Provider name reported on the resulting `AssistantMessage.provider`.
    pub provider_name: String,
    /// Model id sent in the request body.
    pub model: String,
    /// API key sent as `Authorization: Bearer <key>` unless `auth_header_name`
    /// is overridden.
    pub api_key: String,
    /// Optional header name. Defaults to `"Authorization"` with `Bearer ` prefix.
    pub auth_header_name: Option<String>,
    /// Optional auth value prefix. Defaults to `"Bearer "`.
    pub auth_value_prefix: Option<String>,
    /// Extra headers to send (e.g. `HTTP-Referer` for OpenRouter).
    pub extra_headers: Vec<(String, String)>,
    /// Maximum tokens to request. Default 4096.
    pub max_tokens: u32,
}

impl ChatCompletionsConfig {
    pub fn new(
        url: impl Into<String>,
        provider_name: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            provider_name: provider_name.into(),
            model: model.into(),
            api_key: api_key.into(),
            auth_header_name: None,
            auth_value_prefix: None,
            extra_headers: Vec::new(),
            max_tokens: 4096,
        }
    }

    pub fn with_max_tokens(mut self, max: u32) -> Self {
        self.max_tokens = max;
        self
    }

    pub fn with_extra_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }
}

/// Wire-shape conversion: harness `AgentMessage[]` -> OpenAI `messages` array.
pub fn to_openai_messages(
    messages: &[AgentMessage],
    system_prompt: &str,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if !system_prompt.is_empty() {
        out.push(serde_json::json!({ "role": "system", "content": system_prompt }));
    }
    for m in messages {
        match m {
            AgentMessage::User(u) => {
                let text: String = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(serde_json::json!({ "role": "user", "content": text }));
            }
            AgentMessage::Assistant(a) => {
                let text: String = a
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let tool_calls: Vec<serde_json::Value> = a
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": arguments.to_string(),
                            }
                        })),
                        _ => None,
                    })
                    .collect();
                let mut entry = serde_json::json!({ "role": "assistant" });
                if !text.is_empty() {
                    entry["content"] = serde_json::Value::String(text);
                }
                if !tool_calls.is_empty() {
                    entry["tool_calls"] = serde_json::Value::Array(tool_calls);
                }
                out.push(entry);
            }
            AgentMessage::ToolResult(t) => {
                let text: String = t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text(tx) => Some(tx.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": t.tool_call_id,
                    "content": text,
                }));
            }
            AgentMessage::Custom(_) => {}
        }
    }
    out
}

/// Tool definitions in OpenAI wire shape.
pub fn tools_to_openai(tools: &[AgentTool]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect()
}

/// Build the standard OpenAI Chat Completions request body.
#[derive(Debug, Clone)]
pub struct OpenAICompatRequest {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentTool>,
}

/// Stream a Chat Completions response. Implementations of providers using the
/// OpenAI-compat wire shape call this directly.
pub async fn stream_chat_completions(
    cfg: Arc<ChatCompletionsConfig>,
    request: OpenAICompatRequest,
) -> ReceiverStream<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        if let Err(e) = stream_inner(cfg.clone(), request, tx.clone()).await {
            let _ = tx
                .send(error_event(
                    e.to_string(),
                    None,
                    cfg.model.clone(),
                    cfg.provider_name.clone(),
                ))
                .await;
        }
    });
    ReceiverStream::new(rx)
}

#[derive(Debug, Default)]
struct PartialState {
    text: String,
    tool_calls: Vec<PartialToolCall>,
    usage: Usage,
    stop_reason: Option<StopReason>,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args_json: String,
}

async fn stream_inner(
    cfg: Arc<ChatCompletionsConfig>,
    request: OpenAICompatRequest,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), reqwest::Error> {
    let mut body = serde_json::json!({
        "model": cfg.model,
        "max_tokens": cfg.max_tokens,
        "messages": to_openai_messages(&request.messages, &request.system_prompt),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !request.tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools_to_openai(&request.tools));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let auth_name = cfg.auth_header_name.as_deref().unwrap_or("Authorization");
    let auth_prefix = cfg.auth_value_prefix.as_deref().unwrap_or("Bearer ");
    let mut req = client
        .post(&cfg.url)
        .header("content-type", "application/json")
        .header(auth_name, format!("{auth_prefix}{}", cfg.api_key));
    for (name, value) in &cfg.extra_headers {
        req = req.header(name, value);
    }
    let resp = req.json(&body).send().await?;

    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        let _ = tx
            .send(error_event(
                err_text,
                Some(status.as_u16()),
                cfg.model.clone(),
                cfg.provider_name.clone(),
            ))
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
        provider: cfg.provider_name.clone(),
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
        let chunk: Bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                let _ = tx
                    .send(error_event(
                        e.to_string(),
                        None,
                        cfg.model.clone(),
                        cfg.provider_name.clone(),
                    ))
                    .await;
                return Ok(());
            }
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find("\n\n") {
            let block = buf[..idx].to_string();
            buf.drain(..=idx + 1);
            if let Some(event) = parse_sse_block(&block) {
                if event.data == "[DONE]" {
                    break;
                }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    handle_chunk(&parsed, &mut state, &tx, &cfg).await;
                }
            }
        }
    }

    let final_msg = build_final(&state, &cfg.model, &cfg.provider_name);
    let _ = tx
        .send(AssistantMessageEvent::Done { message: final_msg })
        .await;
    Ok(())
}

async fn handle_chunk(
    chunk: &serde_json::Value,
    state: &mut PartialState,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    cfg: &ChatCompletionsConfig,
) {
    if let Some(usage) = chunk.get("usage") {
        merge_usage(usage, &mut state.usage);
    }
    let Some(choice) = chunk
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return;
    };

    if let Some(finish) = choice.get("finish_reason").and_then(|v| v.as_str()) {
        state.stop_reason = Some(map_finish_reason(finish));
    }

    let Some(delta) = choice.get("delta") else {
        return;
    };

    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
        if state.text.is_empty() {
            let _ = tx
                .send(AssistantMessageEvent::TextStart {
                    partial: build_partial(state, &cfg.model, &cfg.provider_name),
                })
                .await;
        }
        state.text.push_str(text);
        let _ = tx
            .send(AssistantMessageEvent::TextDelta {
                partial: build_partial(state, &cfg.model, &cfg.provider_name),
                delta: text.to_string(),
            })
            .await;
    }

    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            while state.tool_calls.len() <= index {
                state.tool_calls.push(PartialToolCall::default());
            }
            let entry = &mut state.tool_calls[index];
            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    entry.id = id.to_string();
                }
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    if !name.is_empty() {
                        entry.name = name.to_string();
                    }
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.args_json.push_str(args);
                    let _ = tx
                        .send(AssistantMessageEvent::ToolcallDelta {
                            partial: build_partial(state, &cfg.model, &cfg.provider_name),
                            delta: args.to_string(),
                        })
                        .await;
                }
            }
        }
    }
}

fn merge_usage(usage: &serde_json::Value, into: &mut Usage) {
    if let Some(v) = usage
        .get("prompt_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.input += v;
    }
    if let Some(v) = usage
        .get("completion_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.output += v;
    }
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
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::End,
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::Tool,
        _ => StopReason::End,
    }
}

fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    let mut out = Vec::new();
    if !state.text.is_empty() {
        out.push(ContentBlock::Text(TextContent {
            text: state.text.clone(),
        }));
    }
    for tc in &state.tool_calls {
        if tc.name.is_empty() {
            continue;
        }
        let args = if tc.args_json.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str::<serde_json::Value>(&tc.args_json)
                .unwrap_or(serde_json::Value::Null)
        };
        out.push(ContentBlock::ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: args,
        });
    }
    out
}

fn build_partial(state: &PartialState, model: &str, provider: &str) -> AssistantMessage {
    AssistantMessage {
        content: build_content(state),
        stop_reason: state.stop_reason.unwrap_or(StopReason::End),
        error_message: None,
        error_kind: None,
        usage: Some(state.usage),
        model: model.to_string(),
        provider: provider.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

fn build_final(state: &PartialState, model: &str, provider: &str) -> AssistantMessage {
    build_partial(state, model, provider)
}

#[allow(dead_code)]
fn _kept(_: ToolCall, _: fn(&str, Option<u16>) -> harness_types::ErrorKind) {
    let _ = classify_provider_error;
}
