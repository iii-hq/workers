//! Streaming client for the OpenAI Responses API.
//!
//! Implements the `StreamFn` contract used by the harness loop: never throws,
//! always returns an event-yielding stream that ends with `done` or `error`.
//!
//! Unlike Chat Completions, the Responses API uses a typed event stream over
//! SSE: `response.created`, `response.output_text.delta`,
//! `response.function_call_arguments.delta`, `response.completed`, etc. Tool
//! calls land as `output[].type == "function_call"` items and arguments stream
//! as a JSON string. Because the wire shape diverges materially from Chat
//! Completions, this crate does not delegate to `provider_base::stream_chat_completions`;
//! it implements the SSE loop directly while still using `provider_base` for
//! event-block parsing and error encoding.
//!
//! Scope for 0.1.x: text and tool-use output items; usage; basic stop-reason
//! mapping. Reasoning items, refusal deltas, and service-tier pricing land in
//! later passes alongside the Anthropic feature parity work.

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use harness_types::{
    AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind,
    StopReason, TextContent, Usage,
};
use provider_base::{error_event, parse_sse_block};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Default Responses API endpoint.
pub const DEFAULT_API_URL: &str = "https://api.openai.com/v1/responses";

/// Provider name reported on every emitted `AssistantMessage`.
pub const PROVIDER_NAME: &str = "openai-responses";

#[derive(Debug, Error)]
pub enum OpenAIResponsesError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
}

/// Configuration for a single Responses API streaming call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenAIResponsesConfig {
    /// Header-bearer credential. Accepts either an API key or an OAuth
    /// access token — both are sent verbatim as `Authorization: Bearer
    /// <value>`. Populated via [`OpenAIResponsesConfig::with_credential`]
    /// in P5; the legacy [`OpenAIResponsesConfig::from_env`] constructor
    /// still reads the env var directly.
    pub api_key: String,
    pub model: String,
    pub max_output_tokens: u32,
    pub api_url: String,
}

impl OpenAIResponsesConfig {
    /// Build a config from `OPENAI_API_KEY`. Defaults `max_output_tokens` to
    /// 4096 and `api_url` to [`DEFAULT_API_URL`].
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let key = std::env::var("OPENAI_API_KEY")?;
        Ok(Self {
            api_key: key,
            model: model.into(),
            max_output_tokens: 4096,
            api_url: DEFAULT_API_URL.into(),
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// Both `Credential::ApiKey` and `Credential::OAuth` collapse into the
    /// same Bearer header.
    pub fn with_credential(
        model: impl Into<String>,
        cred: &auth_credentials::Credential,
    ) -> anyhow::Result<Self> {
        let key = match cred {
            auth_credentials::Credential::ApiKey { key } => key.clone(),
            auth_credentials::Credential::OAuth { access_token, .. } => access_token.clone(),
        };
        Ok(Self {
            api_key: key,
            model: model.into(),
            max_output_tokens: 4096,
            api_url: DEFAULT_API_URL.into(),
        })
    }

    pub fn with_max_output_tokens(mut self, max: u32) -> Self {
        self.max_output_tokens = max;
        self
    }

    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }
}

/// Convert harness messages into Responses API `input` entries. The Responses
/// API treats each entry as either a role-tagged message or a typed item like
/// `function_call` / `function_call_output`.
pub fn to_responses_input(
    messages: &[AgentMessage],
    system_prompt: &str,
    reasoning_model: bool,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if !system_prompt.is_empty() {
        // Reasoning models use the `developer` role; classic chat models use `system`.
        let role = if reasoning_model {
            "developer"
        } else {
            "system"
        };
        out.push(serde_json::json!({
            "role": role,
            "content": system_prompt,
        }));
    }
    for m in messages {
        match m {
            AgentMessage::User(u) => {
                let content: Vec<serde_json::Value> = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text(t) => Some(serde_json::json!({
                            "type": "input_text",
                            "text": t.text,
                        })),
                        _ => None,
                    })
                    .collect();
                if content.is_empty() {
                    continue;
                }
                out.push(serde_json::json!({
                    "role": "user",
                    "content": content,
                }));
            }
            AgentMessage::Assistant(a) => {
                for block in &a.content {
                    match block {
                        ContentBlock::Text(t) => {
                            out.push(serde_json::json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{
                                    "type": "output_text",
                                    "text": t.text,
                                    "annotations": [],
                                }],
                                "status": "completed",
                            }));
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            out.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": arguments.to_string(),
                            }));
                        }
                        _ => {}
                    }
                }
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
                    "type": "function_call_output",
                    "call_id": t.tool_call_id,
                    "output": text,
                }));
            }
            AgentMessage::Custom(_) => {}
        }
    }
    out
}

/// Tool definitions in the Responses API wire shape.
pub fn tools_to_responses(tools: &[AgentTool]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect()
}

/// Stream a response from the OpenAI Responses API. Returns an event stream
/// that closes with `done` on success or `error` on failure. Never throws.
pub async fn stream(
    cfg: Arc<OpenAIResponsesConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
) -> ReceiverStream<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        if let Err(e) = stream_inner(cfg.clone(), system_prompt, messages, tools, tx.clone()).await
        {
            let _ = tx
                .send(error_event(
                    e.to_string(),
                    None,
                    cfg.model.clone(),
                    PROVIDER_NAME,
                ))
                .await;
        }
    });
    ReceiverStream::new(rx)
}

#[derive(Debug, Default)]
struct PartialState {
    /// One text block per `message` output item.
    text_blocks: Vec<String>,
    /// One tool-call entry per `function_call` output item.
    tool_calls: Vec<PartialToolCall>,
    usage: Usage,
    stop_reason: Option<StopReason>,
    /// Tracks the kind of the currently-open output item so deltas route to
    /// the right block.
    current_item: Option<OutputItemKind>,
}

#[derive(Debug, Clone, Copy)]
enum OutputItemKind {
    Message,
    FunctionCall,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args_json: String,
}

async fn stream_inner(
    cfg: Arc<OpenAIResponsesConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), OpenAIResponsesError> {
    let mut body = serde_json::json!({
        "model": cfg.model,
        "input": to_responses_input(&messages, &system_prompt, false),
        "stream": true,
        "max_output_tokens": cfg.max_output_tokens,
        "store": false,
    });
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools_to_responses(&tools));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_mins(2))
        .build()?;

    let resp = client
        .post(&cfg.api_url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", cfg.api_key))
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        let _ = tx
            .send(error_event(
                err_text,
                Some(status.as_u16()),
                cfg.model.clone(),
                PROVIDER_NAME,
            ))
            .await;
        return Ok(());
    }

    let initial = AssistantMessage {
        content: Vec::new(),
        stop_reason: StopReason::End,
        error_message: None,
        error_kind: None,
        usage: None,
        model: cfg.model.clone(),
        provider: PROVIDER_NAME.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    let _ = tx
        .send(AssistantMessageEvent::Start { partial: initial })
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
                        PROVIDER_NAME,
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
                    handle_response_event(&parsed, &mut state, &tx, &cfg.model).await;
                }
            }
        }
    }

    let final_msg = build_final(&state, &cfg.model);
    let _ = tx
        .send(AssistantMessageEvent::Done { message: final_msg })
        .await;
    Ok(())
}

async fn handle_response_event(
    event: &serde_json::Value,
    state: &mut PartialState,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    model: &str,
) {
    let Some(event_type) = event.get("type").and_then(|v| v.as_str()) else {
        return;
    };

    match event_type {
        "response.output_item.added" => {
            let item = event.get("item");
            let item_type = item.and_then(|i| i.get("type")).and_then(|v| v.as_str());
            match item_type {
                Some("message") => {
                    state.text_blocks.push(String::new());
                    state.current_item = Some(OutputItemKind::Message);
                    let _ = tx
                        .send(AssistantMessageEvent::TextStart {
                            partial: build_partial(state, model),
                        })
                        .await;
                }
                Some("function_call") => {
                    let call_id = item
                        .and_then(|i| i.get("call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .and_then(|i| i.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let initial_args = item
                        .and_then(|i| i.get("arguments"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    state.tool_calls.push(PartialToolCall {
                        id: call_id,
                        name,
                        args_json: initial_args,
                    });
                    state.current_item = Some(OutputItemKind::FunctionCall);
                    let _ = tx
                        .send(AssistantMessageEvent::ToolcallStart {
                            partial: build_partial(state, model),
                        })
                        .await;
                }
                _ => {
                    // Reasoning and other items are intentionally dropped at this scope.
                    state.current_item = None;
                }
            }
        }
        "response.output_text.delta" => {
            let delta = event
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(last) = state.text_blocks.last_mut() {
                last.push_str(&delta);
            }
            let _ = tx
                .send(AssistantMessageEvent::TextDelta {
                    partial: build_partial(state, model),
                    delta,
                })
                .await;
        }
        "response.output_text.done" => {
            if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                if let Some(last) = state.text_blocks.last_mut() {
                    *last = text.to_string();
                }
            }
            let _ = tx
                .send(AssistantMessageEvent::TextEnd {
                    partial: build_partial(state, model),
                })
                .await;
        }
        "response.function_call_arguments.delta" => {
            let delta = event
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(last) = state.tool_calls.last_mut() {
                last.args_json.push_str(&delta);
            }
            let _ = tx
                .send(AssistantMessageEvent::ToolcallDelta {
                    partial: build_partial(state, model),
                    delta,
                })
                .await;
        }
        "response.function_call_arguments.done" => {
            if let Some(args) = event.get("arguments").and_then(|v| v.as_str()) {
                if let Some(last) = state.tool_calls.last_mut() {
                    last.args_json = args.to_string();
                }
            }
            let _ = tx
                .send(AssistantMessageEvent::ToolcallEnd {
                    partial: build_partial(state, model),
                })
                .await;
        }
        "response.output_item.done" => {
            // Some servers omit the explicit `output_text.done` event, so we
            // also close text blocks at item boundaries when needed.
            state.current_item = None;
        }
        "response.completed" => {
            if let Some(response) = event.get("response") {
                if let Some(usage) = response.get("usage") {
                    merge_usage(usage, &mut state.usage);
                }
                if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
                    state.stop_reason = Some(map_status(status));
                }
            }
            // If a tool call was emitted but the status indicates a normal
            // stop, upgrade to Tool so the loop runs the tool turn.
            if !state.tool_calls.is_empty()
                && matches!(state.stop_reason, Some(StopReason::End) | None)
            {
                state.stop_reason = Some(StopReason::Tool);
            }
        }
        "response.failed" => {
            let msg = event
                .get("response")
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("response.failed")
                .to_string();
            state.stop_reason = Some(StopReason::Error);
            let _ = tx
                .send(error_event(msg, None, model.to_string(), PROVIDER_NAME))
                .await;
        }
        "error" => {
            let msg = event
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("error")
                .to_string();
            state.stop_reason = Some(StopReason::Error);
            let _ = tx
                .send(error_event(msg, None, model.to_string(), PROVIDER_NAME))
                .await;
        }
        _ => {}
    }
}

fn merge_usage(usage: &serde_json::Value, into: &mut Usage) {
    let cached = usage
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if let Some(v) = usage
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        // OpenAI counts cached tokens inside input_tokens; subtract so we
        // only attribute fresh input here and report cache reads separately.
        into.input += v.saturating_sub(cached);
    }
    if let Some(v) = usage
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        into.output += v;
    }
    into.cache_read += cached;
}

fn map_status(status: &str) -> StopReason {
    match status {
        "completed" => StopReason::End,
        "incomplete" => StopReason::Length,
        "failed" | "cancelled" => StopReason::Error,
        _ => StopReason::End,
    }
}

fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    let mut out = Vec::new();
    for t in &state.text_blocks {
        if !t.is_empty() {
            out.push(ContentBlock::Text(TextContent { text: t.clone() }));
        }
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

fn build_partial(state: &PartialState, model: &str) -> AssistantMessage {
    AssistantMessage {
        content: build_content(state),
        stop_reason: state.stop_reason.unwrap_or(StopReason::End),
        error_message: None,
        error_kind: None,
        usage: Some(state.usage),
        model: model.to_string(),
        provider: PROVIDER_NAME.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

fn build_final(state: &PartialState, model: &str) -> AssistantMessage {
    build_partial(state, model)
}

/// Register `provider::openai-responses::complete` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<OpenAIResponsesConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            OpenAIResponsesConfig::with_credential(model, cred)
        },
        stream,
    );
    Ok(())
}

/// Convenience: collect a stream into a final `AssistantMessage`.
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
        content: vec![ContentBlock::Text(TextContent {
            text: "stream closed without final".into(),
        })],
        stop_reason: StopReason::Error,
        error_message: Some("stream closed without final".into()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: String::new(),
        provider: PROVIDER_NAME.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{AgentMessage, ContentBlock, TextContent, ToolResultMessage, UserMessage};

    #[test]
    fn from_env_reads_openai_api_key() {
        let prev = std::env::var("OPENAI_API_KEY").ok();
        std::env::set_var("OPENAI_API_KEY", "sk-test-fixture");
        let cfg = OpenAIResponsesConfig::from_env("gpt-5").expect("env present");
        assert_eq!(cfg.api_key, "sk-test-fixture");
        assert_eq!(cfg.model, "gpt-5");
        assert_eq!(cfg.max_output_tokens, 4096);
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
        match prev {
            Some(v) => std::env::set_var("OPENAI_API_KEY", v),
            None => std::env::remove_var("OPENAI_API_KEY"),
        }
    }

    #[test]
    fn with_credential_api_key() {
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-test".into(),
        };
        let cfg = OpenAIResponsesConfig::with_credential("gpt-5", &cred).unwrap();
        assert_eq!(cfg.api_key, "sk-test");
        assert_eq!(cfg.model, "gpt-5");
    }

    #[test]
    fn with_credential_oauth_uses_access_token() {
        let cred = auth_credentials::Credential::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            provider_extra: serde_json::Value::Null,
        };
        let cfg = OpenAIResponsesConfig::with_credential("gpt-5", &cred).unwrap();
        assert_eq!(cfg.api_key, "tok");
    }

    #[test]
    fn user_message_converts_to_input_text() {
        let msgs = vec![AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: "hi there".into(),
            })],
            timestamp: 1,
        })];
        let wire = to_responses_input(&msgs, "be terse", false);
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[1]["role"], "user");
        assert_eq!(wire[1]["content"][0]["type"], "input_text");
        assert_eq!(wire[1]["content"][0]["text"], "hi there");
    }

    #[test]
    fn reasoning_model_uses_developer_role() {
        let wire = to_responses_input(&[], "instructions", true);
        assert_eq!(wire[0]["role"], "developer");
    }

    #[test]
    fn tool_result_converts_to_function_call_output() {
        let msgs = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "call_1".into(),
            tool_name: "read".into(),
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            details: serde_json::json!({}),
            is_error: false,
            timestamp: 2,
        })];
        let wire = to_responses_input(&msgs, "", false);
        assert_eq!(wire[0]["type"], "function_call_output");
        assert_eq!(wire[0]["call_id"], "call_1");
        assert_eq!(wire[0]["output"], "ok");
    }

    #[test]
    fn tools_use_top_level_function_shape() {
        let tools = vec![AgentTool {
            name: "read".into(),
            description: "read a file".into(),
            parameters: serde_json::json!({"type": "object"}),
            label: "Read".into(),
            execution_mode: harness_types::ExecutionMode::default(),
            prepare_arguments_supported: false,
        }];
        let wire = tools_to_responses(&tools);
        assert_eq!(wire[0]["type"], "function");
        assert_eq!(wire[0]["name"], "read");
        assert_eq!(wire[0]["parameters"]["type"], "object");
    }

    #[test]
    fn map_status_known_values() {
        assert!(matches!(map_status("completed"), StopReason::End));
        assert!(matches!(map_status("incomplete"), StopReason::Length));
        assert!(matches!(map_status("failed"), StopReason::Error));
        assert!(matches!(map_status("cancelled"), StopReason::Error));
    }

    #[test]
    fn merge_usage_subtracts_cached_from_input() {
        let mut u = Usage::default();
        merge_usage(
            &serde_json::json!({
                "input_tokens": 100,
                "output_tokens": 50,
                "input_tokens_details": { "cached_tokens": 30 },
            }),
            &mut u,
        );
        assert_eq!(u.input, 70);
        assert_eq!(u.output, 50);
        assert_eq!(u.cache_read, 30);
    }

    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY"]
    async fn live_stream_smoke() {
        if std::env::var("OPENAI_API_KEY").is_err() {
            return;
        }
        let cfg = Arc::new(OpenAIResponsesConfig::from_env("gpt-4o-mini").unwrap());
        let s = stream(cfg, "You are terse.".into(), Vec::new(), Vec::new()).await;
        let msg = collect(s).await;
        assert_eq!(msg.provider, PROVIDER_NAME);
    }
}
