//! Streaming client for the Google Gemini Generative Language API.
//!
//! Implements the `StreamFn` contract used by the harness loop: never throws,
//! always returns an event-yielding stream that ends with `done` or `error`.
//!
//! Endpoint shape:
//! `POST {api_url}/{model}:streamGenerateContent?alt=sse`
//! with header `x-goog-api-key: <GOOGLE_API_KEY>`. The response is an SSE
//! stream where each `data:` line is a JSON object containing zero or more
//! `candidates[].content.parts[]` items. Parts may be `{text}` or
//! `{functionCall: {name, args}}`. A `usageMetadata` block at the end
//! reports token counts; a `finishReason` triggers stop mapping.
//!
//! Scope for 0.1.x: text and function-call parts; no inline image parts;
//! no safety-rating surfacing.

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use harness_types::{
    AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind,
    StopReason, TextContent, Usage,
};
use provider_base::{classify_provider_error, error_event, parse_sse_block};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Default Generative Language API base. Append `/{model}:streamGenerateContent`.
pub const DEFAULT_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Provider name reported on every emitted `AssistantMessage`.
pub const PROVIDER_NAME: &str = "google";

#[derive(Debug, Error)]
pub enum GoogleError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuration for a single Gemini streaming call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GoogleConfig {
    /// Header credential. Sent as `x-goog-api-key: <value>` (or as a
    /// `?key=<value>` query param). Accepts either an API key or an OAuth
    /// access token — both are forwarded verbatim. Populated via
    /// [`GoogleConfig::with_credential`] in P5; the legacy
    /// [`GoogleConfig::from_env`] constructor still reads the env var
    /// directly.
    pub api_key: String,
    pub model: String,
    pub max_output_tokens: u32,
    pub api_url: String,
}

impl GoogleConfig {
    /// Build a config from `GOOGLE_API_KEY`. Defaults `max_output_tokens` to
    /// 4096 and `api_url` to [`DEFAULT_API_URL`].
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let key = std::env::var("GOOGLE_API_KEY")?;
        Ok(Self {
            api_key: key,
            model: model.into(),
            max_output_tokens: 4096,
            api_url: DEFAULT_API_URL.into(),
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// Both `Credential::ApiKey` and `Credential::OAuth` collapse into the
    /// same `x-goog-api-key` header value (Google accepts an OAuth access
    /// token as that header for tier-1 endpoints).
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

/// Convert harness `AgentMessage`s into Gemini `contents[]` entries.
///
/// - `User` becomes `{ role: "user", parts: [{text}, ...] }`.
/// - `Assistant` becomes `{ role: "model", parts: [{text}|{functionCall}, ...] }`.
/// - `ToolResult` becomes `{ role: "user", parts: [{functionResponse}] }`.
/// - `Custom` is skipped.
pub fn to_wire_contents(messages: &[AgentMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in messages {
        match m {
            AgentMessage::User(u) => {
                let parts = u
                    .content
                    .iter()
                    .filter_map(content_block_to_part)
                    .collect::<Vec<_>>();
                if !parts.is_empty() {
                    out.push(serde_json::json!({ "role": "user", "parts": parts }));
                }
            }
            AgentMessage::Assistant(a) => {
                let parts = a
                    .content
                    .iter()
                    .filter_map(content_block_to_part)
                    .collect::<Vec<_>>();
                if !parts.is_empty() {
                    out.push(serde_json::json!({ "role": "model", "parts": parts }));
                }
            }
            AgentMessage::ToolResult(t) => {
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
                    "parts": [{
                        "functionResponse": {
                            "name": t.tool_name,
                            "response": { "content": text },
                        }
                    }]
                }));
            }
            AgentMessage::Custom(_) => {}
        }
    }
    out
}

fn content_block_to_part(b: &ContentBlock) -> Option<Value> {
    match b {
        ContentBlock::Text(t) => Some(serde_json::json!({ "text": t.text })),
        ContentBlock::ToolCall {
            name, arguments, ..
        } => Some(serde_json::json!({
            "functionCall": { "name": name, "args": arguments }
        })),
        _ => None,
    }
}

/// Tool definitions in Gemini wire shape: a single `tools[]` entry that wraps
/// every harness tool as a `functionDeclarations` element.
pub fn tools_to_wire(tools: &[AgentTool]) -> Vec<Value> {
    if tools.is_empty() {
        return Vec::new();
    }
    let decls = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect::<Vec<_>>();
    vec![serde_json::json!({ "functionDeclarations": decls })]
}

/// Map a Gemini `finishReason` string onto the harness [`StopReason`].
pub fn map_finish_reason(s: &str) -> StopReason {
    match s {
        "STOP" => StopReason::End,
        "MAX_TOKENS" => StopReason::Length,
        _ => StopReason::End,
    }
}

/// Stream a response from Gemini. Returns an event stream that closes with
/// `done` on success or `error` on failure. Never throws.
pub async fn stream(
    cfg: Arc<GoogleConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
) -> ReceiverStream<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    let model_for_err = cfg.model.clone();
    tokio::spawn(async move {
        if let Err(e) = stream_inner(cfg, system_prompt, messages, tools, tx.clone()).await {
            let _ = tx
                .send(error_event(
                    e.to_string(),
                    None,
                    model_for_err,
                    PROVIDER_NAME,
                ))
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
    last_text_open: bool,
    last_tool_open: bool,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args: Value,
}

async fn stream_inner(
    cfg: Arc<GoogleConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), GoogleError> {
    let url = format!(
        "{}/{}:streamGenerateContent?alt=sse",
        cfg.api_url.trim_end_matches('/'),
        cfg.model
    );
    let wire_tools = tools_to_wire(&tools);
    let mut body = serde_json::json!({
        "contents": to_wire_contents(&messages),
        "generationConfig": {
            "maxOutputTokens": cfg.max_output_tokens,
        },
    });
    if !system_prompt.is_empty() {
        body["systemInstruction"] = serde_json::json!({
            "parts": [{ "text": system_prompt }]
        });
    }
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let resp = client
        .post(&url)
        .header("x-goog-api-key", &cfg.api_key)
        .header("content-type", "application/json")
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

    let partial_msg = AssistantMessage {
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
        .send(AssistantMessageEvent::Start {
            partial: partial_msg,
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
            let event_block = buf[..idx].to_string();
            buf.drain(..=idx + 1);
            if let Some(ev) = parse_sse_block(&event_block) {
                handle_data(&ev.data, &mut state, &tx, &cfg.model).await;
            }
        }
    }

    // Close any still-open block.
    close_open_blocks(&mut state, &tx, &cfg.model).await;

    let _ = tx
        .send(AssistantMessageEvent::Stop {
            stop_reason: state.stop_reason.unwrap_or(StopReason::End),
            error_message: None,
            error_kind: None,
        })
        .await;

    let final_message = build_partial(&state, &cfg.model);
    let _ = tx
        .send(AssistantMessageEvent::Done {
            message: final_message,
        })
        .await;
    Ok(())
}

async fn handle_data(
    data: &str,
    state: &mut PartialState,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    model: &str,
) {
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return;
    };

    // usageMetadata may arrive on any chunk.
    if let Some(usage) = parsed.get("usageMetadata") {
        merge_usage(usage, &mut state.usage);
    }

    if let Some(err) = parsed.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("gemini error");
        let kind = classify_provider_error(msg, None);
        let final_msg = AssistantMessage {
            content: vec![ContentBlock::Text(TextContent { text: msg.into() })],
            stop_reason: StopReason::Error,
            error_message: Some(msg.into()),
            error_kind: Some(kind),
            usage: Some(state.usage),
            model: model.into(),
            provider: PROVIDER_NAME.into(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };
        let _ = tx
            .send(AssistantMessageEvent::Error { error: final_msg })
            .await;
        return;
    }

    let Some(candidates) = parsed.get("candidates").and_then(Value::as_array) else {
        return;
    };

    for cand in candidates {
        if let Some(parts) = cand
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
        {
            for part in parts {
                handle_part(part, state, tx, model).await;
            }
        }

        if let Some(finish) = cand.get("finishReason").and_then(Value::as_str) {
            state.stop_reason = Some(map_finish_reason(finish));
        }
    }
}

async fn handle_part(
    part: &Value,
    state: &mut PartialState,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    model: &str,
) {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        // Close any open tool block before opening text.
        if state.last_tool_open {
            let _ = tx
                .send(AssistantMessageEvent::ToolcallEnd {
                    partial: build_partial(state, model),
                })
                .await;
            state.last_tool_open = false;
        }
        if !state.last_text_open {
            state.text_blocks.push(String::new());
            state.last_text_open = true;
            let _ = tx
                .send(AssistantMessageEvent::TextStart {
                    partial: build_partial(state, model),
                })
                .await;
        }
        if let Some(last) = state.text_blocks.last_mut() {
            last.push_str(text);
        }
        let _ = tx
            .send(AssistantMessageEvent::TextDelta {
                partial: build_partial(state, model),
                delta: text.to_string(),
            })
            .await;
        return;
    }

    if let Some(fc) = part.get("functionCall") {
        if state.last_text_open {
            let _ = tx
                .send(AssistantMessageEvent::TextEnd {
                    partial: build_partial(state, model),
                })
                .await;
            state.last_text_open = false;
        }
        let name = fc
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = fc
            .get("args")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let id = format!("call_{}_{}", state.tool_calls.len(), name);
        state.tool_calls.push(PartialToolCall {
            id,
            name,
            args: args.clone(),
        });
        state.last_tool_open = true;
        let _ = tx
            .send(AssistantMessageEvent::ToolcallStart {
                partial: build_partial(state, model),
            })
            .await;
        let delta = serde_json::to_string(&args).unwrap_or_default();
        let _ = tx
            .send(AssistantMessageEvent::ToolcallDelta {
                partial: build_partial(state, model),
                delta,
            })
            .await;
        let _ = tx
            .send(AssistantMessageEvent::ToolcallEnd {
                partial: build_partial(state, model),
            })
            .await;
        state.last_tool_open = false;
        if state.stop_reason == Some(StopReason::End) {
            state.stop_reason = Some(StopReason::Tool);
        }
    }
}

async fn close_open_blocks(
    state: &mut PartialState,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    model: &str,
) {
    if state.last_text_open {
        let _ = tx
            .send(AssistantMessageEvent::TextEnd {
                partial: build_partial(state, model),
            })
            .await;
        state.last_text_open = false;
    }
    if state.last_tool_open {
        let _ = tx
            .send(AssistantMessageEvent::ToolcallEnd {
                partial: build_partial(state, model),
            })
            .await;
        state.last_tool_open = false;
    }
}

fn merge_usage(usage: &Value, into: &mut Usage) {
    if let Some(v) = usage.get("promptTokenCount").and_then(Value::as_u64) {
        into.input = v;
    }
    if let Some(v) = usage.get("candidatesTokenCount").and_then(Value::as_u64) {
        into.output = v;
    }
    if let Some(v) = usage.get("cachedContentTokenCount").and_then(Value::as_u64) {
        into.cache_read = v;
    }
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

fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    let mut content = Vec::new();
    for t in &state.text_blocks {
        if !t.is_empty() {
            content.push(ContentBlock::Text(TextContent { text: t.clone() }));
        }
    }
    for tc in &state.tool_calls {
        content.push(ContentBlock::ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.args.clone(),
        });
    }
    content
}

/// Register `provider::google::complete` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<GoogleConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            GoogleConfig::with_credential(model, cred)
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
    use harness_types::{
        AgentMessage, AssistantMessage, ContentBlock, TextContent, ToolResultMessage, UserMessage,
    };

    #[test]
    fn user_message_converts_to_user_role_with_text_part() {
        let msgs = vec![AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: "hello".into(),
            })],
            timestamp: 1,
        })];
        let wire = to_wire_contents(&msgs);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[0]["parts"][0]["text"], "hello");
    }

    #[test]
    fn assistant_with_tool_call_maps_to_model_role_with_function_call_part() {
        let msgs = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![
                ContentBlock::Text(TextContent {
                    text: "calling".into(),
                }),
                ContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({ "path": "/tmp/x" }),
                },
            ],
            stop_reason: StopReason::Tool,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "gemini".into(),
            provider: PROVIDER_NAME.into(),
            timestamp: 1,
        })];
        let wire = to_wire_contents(&msgs);
        assert_eq!(wire[0]["role"], "model");
        assert_eq!(wire[0]["parts"][0]["text"], "calling");
        assert_eq!(wire[0]["parts"][1]["functionCall"]["name"], "read");
        assert_eq!(
            wire[0]["parts"][1]["functionCall"]["args"]["path"],
            "/tmp/x"
        );
    }

    #[test]
    fn tool_result_maps_to_user_role_with_function_response_part() {
        let msgs = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc1".into(),
            tool_name: "read".into(),
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            details: serde_json::json!({}),
            is_error: false,
            timestamp: 2,
        })];
        let wire = to_wire_contents(&msgs);
        assert_eq!(wire[0]["role"], "user");
        let fr = &wire[0]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "read");
        assert_eq!(fr["response"]["content"], "ok");
    }

    #[test]
    fn tools_wrap_in_function_declarations_array() {
        let tools = vec![harness_types::AgentTool {
            name: "read".into(),
            description: "read a file".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: "read".into(),
            execution_mode: harness_types::ExecutionMode::Parallel,
            prepare_arguments_supported: false,
        }];
        let wire = tools_to_wire(&tools);
        assert_eq!(wire.len(), 1);
        let decls = &wire[0]["functionDeclarations"];
        assert_eq!(decls[0]["name"], "read");
        assert_eq!(decls[0]["description"], "read a file");
        assert_eq!(decls[0]["parameters"]["type"], "object");
    }

    #[test]
    fn with_credential_api_key() {
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-test".into(),
        };
        let cfg = GoogleConfig::with_credential("the-model", &cred).unwrap();
        assert_eq!(cfg.api_key, "sk-test");
        assert_eq!(cfg.model, "the-model");
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
        let cfg = GoogleConfig::with_credential("the-model", &cred).unwrap();
        assert_eq!(cfg.api_key, "tok");
    }

    #[test]
    fn finish_reason_mapping() {
        assert!(matches!(map_finish_reason("STOP"), StopReason::End));
        assert!(matches!(
            map_finish_reason("MAX_TOKENS"),
            StopReason::Length
        ));
        assert!(matches!(map_finish_reason("SAFETY"), StopReason::End));
    }

    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY"]
    async fn live_smoke() {
        let Ok(cfg) = GoogleConfig::from_env("gemini-1.5-flash") else {
            eprintln!("skipping: GOOGLE_API_KEY unset");
            return;
        };
        let cfg = Arc::new(cfg);
        let s = stream(
            cfg,
            "You are a brevity bot.".into(),
            vec![AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent {
                    text: "Say only the word: pong".into(),
                })],
                timestamp: 0,
            })],
            Vec::new(),
        )
        .await;
        let final_msg = collect(s).await;
        assert!(matches!(
            final_msg.stop_reason,
            StopReason::End | StopReason::Length
        ));
    }
}
