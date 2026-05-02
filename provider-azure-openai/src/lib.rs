//! Streaming client for the OpenAI Responses API hosted on Azure.
//!
//! Wire shape is identical to `provider-openai-responses` (same `input` array,
//! same `tools` shape, same typed SSE event names). Only the URL and auth
//! header differ:
//!
//! - URL: `https://<resource>.openai.azure.com/openai/responses?api-version=<version>`
//! - Auth: `api-key: <AZURE_OPENAI_API_KEY>` header (not `Authorization: Bearer`)
//!
//! Implementation deliberately duplicates the SSE/state code from
//! `provider-openai-responses` for 0.1. Factoring the shared loop into a
//! helper inside `provider-base` is the obvious follow-up; doing it now would
//! reshape both crates and is out of scope for the Azure wiring pass.

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

/// Default Azure Responses API version.
pub const DEFAULT_API_VERSION: &str = "2025-01-01-preview";

/// Provider name reported on every emitted `AssistantMessage`.
pub const PROVIDER_NAME: &str = "azure-openai";

#[derive(Debug, Error)]
pub enum AzureOpenAIError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
}

/// Configuration for a single Azure Responses API streaming call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AzureOpenAIConfig {
    /// Header credential. Sent as `api-key: <value>`. Accepts either an
    /// API key or an OAuth access token — both are forwarded verbatim.
    /// Populated via [`AzureOpenAIConfig::with_credential`] in P5; the
    /// legacy [`AzureOpenAIConfig::from_env`] constructor still reads the
    /// env var directly.
    pub api_key: String,
    /// Resource name (subdomain). Combined as `https://<resource>.openai.azure.com`.
    pub resource: String,
    /// Model deployment name. Acts as the model id on the Azure side and is
    /// what gets reported on every `AssistantMessage`.
    pub deployment: String,
    /// API version query string. Defaults to [`DEFAULT_API_VERSION`].
    pub api_version: String,
    pub max_output_tokens: u32,
}

impl AzureOpenAIConfig {
    /// Build a config from `AZURE_OPENAI_API_KEY` and `AZURE_OPENAI_RESOURCE`.
    /// Caller supplies the deployment name (no good environmental default).
    /// Defaults `max_output_tokens` to 4096 and `api_version` to
    /// [`DEFAULT_API_VERSION`].
    pub fn from_env(deployment: impl Into<String>) -> Result<Self, std::env::VarError> {
        let api_key = std::env::var("AZURE_OPENAI_API_KEY")?;
        let resource = std::env::var("AZURE_OPENAI_RESOURCE")?;
        Ok(Self {
            api_key,
            resource,
            deployment: deployment.into(),
            api_version: DEFAULT_API_VERSION.into(),
            max_output_tokens: 4096,
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// The primary credential (`AZURE_OPENAI_API_KEY` equivalent) flows
    /// through `auth::get_token`; `AZURE_OPENAI_RESOURCE` remains an env
    /// read because the resource subdomain is not part of `Credential`.
    pub fn with_credential(
        deployment: impl Into<String>,
        cred: &auth_credentials::Credential,
    ) -> anyhow::Result<Self> {
        let api_key = match cred {
            auth_credentials::Credential::ApiKey { key } => key.clone(),
            auth_credentials::Credential::OAuth { access_token, .. } => access_token.clone(),
        };
        let resource = std::env::var("AZURE_OPENAI_RESOURCE")
            .map_err(|e| anyhow::anyhow!("missing AZURE_OPENAI_RESOURCE: {e}"))?;
        Ok(Self {
            api_key,
            resource,
            deployment: deployment.into(),
            api_version: DEFAULT_API_VERSION.into(),
            max_output_tokens: 4096,
        })
    }

    pub fn with_max_output_tokens(mut self, max: u32) -> Self {
        self.max_output_tokens = max;
        self
    }

    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Fully-qualified Responses endpoint, including resource and api-version.
    pub fn api_url(&self) -> String {
        format!(
            "https://{}.openai.azure.com/openai/responses?api-version={}",
            self.resource, self.api_version
        )
    }
}

/// Convert harness messages into Responses API `input` entries. Identical to
/// the upstream OpenAI Responses shape.
pub fn to_responses_input(
    messages: &[AgentMessage],
    system_prompt: &str,
    reasoning_model: bool,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if !system_prompt.is_empty() {
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

/// Stream a response from Azure's hosted Responses API. Returns an event
/// stream that closes with `done` on success or `error` on failure. Never
/// throws.
pub async fn stream(
    cfg: Arc<AzureOpenAIConfig>,
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
                    cfg.deployment.clone(),
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
    cfg: Arc<AzureOpenAIConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), AzureOpenAIError> {
    let mut body = serde_json::json!({
        "model": cfg.deployment,
        "input": to_responses_input(&messages, &system_prompt, false),
        "stream": true,
        "max_output_tokens": cfg.max_output_tokens,
        "store": false,
    });
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools_to_responses(&tools));
    }

    let url = cfg.api_url();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .header("api-key", &cfg.api_key)
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
                cfg.deployment.clone(),
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
        model: cfg.deployment.clone(),
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
                        cfg.deployment.clone(),
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
                    handle_response_event(&parsed, &mut state, &tx, &cfg.deployment).await;
                }
            }
        }
    }

    let final_msg = build_final(&state, &cfg.deployment);
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

/// Register `provider::azure-openai::complete` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<AzureOpenAIConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            AzureOpenAIConfig::with_credential(model, cred)
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

    /// Combined env-var test. Cargo runs tests in parallel by default and
    /// other tests inside this binary that touch the same vars would race;
    /// folding them into one test keeps the surface stable.
    #[test]
    fn from_env_behavior_and_api_url_and_deployment() {
        let prev_key = std::env::var("AZURE_OPENAI_API_KEY").ok();
        let prev_res = std::env::var("AZURE_OPENAI_RESOURCE").ok();

        // from_env_requires_both_key_and_resource
        std::env::remove_var("AZURE_OPENAI_API_KEY");
        std::env::remove_var("AZURE_OPENAI_RESOURCE");
        assert!(AzureOpenAIConfig::from_env("dep").is_err());

        std::env::set_var("AZURE_OPENAI_API_KEY", "test-key");
        assert!(AzureOpenAIConfig::from_env("dep").is_err());

        std::env::set_var("AZURE_OPENAI_RESOURCE", "my-resource");
        let cfg = AzureOpenAIConfig::from_env("my-deployment").expect("env present");
        assert_eq!(cfg.api_key, "test-key");
        assert_eq!(cfg.resource, "my-resource");
        assert_eq!(cfg.api_version, DEFAULT_API_VERSION);
        assert_eq!(cfg.max_output_tokens, 4096);

        // from_env_carries_deployment_as_model_id
        assert_eq!(cfg.deployment, "my-deployment");

        // api_url_includes_resource_and_version
        assert_eq!(
            cfg.api_url(),
            "https://my-resource.openai.azure.com/openai/responses?api-version=2025-01-01-preview"
        );

        // with_api_version override threads through to api_url
        let cfg2 = cfg.with_api_version("2024-08-01-preview");
        assert!(cfg2.api_url().ends_with("api-version=2024-08-01-preview"));

        // with_credential reads AZURE_OPENAI_RESOURCE; the api-key flows
        // through Credential. ApiKey + OAuth folded into this same test to
        // share the env snapshot/restore.
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-test".into(),
        };
        let cfg_c = AzureOpenAIConfig::with_credential("my-deployment", &cred).unwrap();
        assert_eq!(cfg_c.api_key, "sk-test");
        assert_eq!(cfg_c.resource, "my-resource");
        assert_eq!(cfg_c.deployment, "my-deployment");
        assert_eq!(cfg_c.api_version, DEFAULT_API_VERSION);

        let cred_oauth = auth_credentials::Credential::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            provider_extra: serde_json::Value::Null,
        };
        let cfg_o = AzureOpenAIConfig::with_credential("my-deployment", &cred_oauth).unwrap();
        assert_eq!(cfg_o.api_key, "tok");

        // with_credential errors when AZURE_OPENAI_RESOURCE is missing.
        std::env::remove_var("AZURE_OPENAI_RESOURCE");
        assert!(AzureOpenAIConfig::with_credential("dep", &cred).is_err());

        match prev_key {
            Some(v) => std::env::set_var("AZURE_OPENAI_API_KEY", v),
            None => std::env::remove_var("AZURE_OPENAI_API_KEY"),
        }
        match prev_res {
            Some(v) => std::env::set_var("AZURE_OPENAI_RESOURCE", v),
            None => std::env::remove_var("AZURE_OPENAI_RESOURCE"),
        }
    }

    #[test]
    fn api_url_format_pure() {
        // Pure formatting test that does not touch env vars.
        let cfg = AzureOpenAIConfig {
            api_key: "k".into(),
            resource: "ext".into(),
            deployment: "d".into(),
            api_version: "2025-01-01-preview".into(),
            max_output_tokens: 1,
        };
        assert_eq!(
            cfg.api_url(),
            "https://ext.openai.azure.com/openai/responses?api-version=2025-01-01-preview"
        );
    }
}
