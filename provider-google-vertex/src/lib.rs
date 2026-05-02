//! Streaming client for Google Vertex AI Gemini.
//!
//! Wire shape is identical to `provider-google` (Gemini Generative API): same
//! `contents[]`, same `tools[].functionDeclarations`, same `usageMetadata`,
//! same SSE event format. Only the endpoint URL and auth header differ:
//!
//! - URL: `https://<region>-aiplatform.googleapis.com/v1/projects/<project>/locations/<region>/publishers/google/models/<model>:streamGenerateContent?alt=sse`
//! - Auth: `Authorization: Bearer <access token>` from Application Default
//!   Credentials. For 0.1, the access token is supplied directly via the
//!   `GOOGLE_VERTEX_ACCESS_TOKEN` env var; the caller is responsible for
//!   refreshing it. Full ADC integration (service-account JSON, metadata
//!   server, gcloud default) lands in a follow-up.
//!
//! Implementation deliberately duplicates the SSE/state code from
//! `provider-google` for 0.1. The shared loop is a clear factoring candidate
//! for `provider-base` once a third Gemini-shaped backend lands.

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

/// Default Vertex AI region.
pub const DEFAULT_REGION: &str = "us-central1";

/// Provider name reported on every emitted `AssistantMessage`.
pub const PROVIDER_NAME: &str = "google-vertex";

#[derive(Debug, Error)]
pub enum VertexError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuration for a single Vertex AI streaming call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VertexConfig {
    /// OAuth 2 access token from ADC, or whatever string `auth::get_token`
    /// returns (e.g. a `GOOGLE_APPLICATION_CREDENTIALS` path resolved by a
    /// future credential helper). Sent as `Authorization: Bearer <value>`.
    /// Caller refreshes when it expires. Populated via
    /// [`VertexConfig::with_credential`] in P5; the legacy
    /// [`VertexConfig::from_env`] constructor still reads the env var
    /// directly.
    pub access_token: String,
    /// GCP project id (not project number).
    pub project: String,
    /// Region; defaults to [`DEFAULT_REGION`].
    pub region: String,
    /// Publisher model id, e.g. `gemini-2.5-flash`.
    pub model: String,
    pub max_output_tokens: u32,
}

impl VertexConfig {
    /// Build a config from `GOOGLE_VERTEX_ACCESS_TOKEN` and
    /// `GOOGLE_VERTEX_PROJECT`. Region is read from `GOOGLE_VERTEX_REGION` if
    /// set, else falls back to [`DEFAULT_REGION`]. Defaults
    /// `max_output_tokens` to 4096.
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let access_token = std::env::var("GOOGLE_VERTEX_ACCESS_TOKEN")?;
        let project = std::env::var("GOOGLE_VERTEX_PROJECT")?;
        let region =
            std::env::var("GOOGLE_VERTEX_REGION").unwrap_or_else(|_| DEFAULT_REGION.to_string());
        Ok(Self {
            access_token,
            project,
            region,
            model: model.into(),
            max_output_tokens: 4096,
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// The primary credential (a `GOOGLE_APPLICATION_CREDENTIALS` path or
    /// an ADC access token) flows through `auth::get_token` and lands in
    /// `access_token`; `GOOGLE_VERTEX_PROJECT` and the optional
    /// `GOOGLE_VERTEX_REGION` remain env reads — they are not part of the
    /// `Credential` shape today.
    pub fn with_credential(
        model: impl Into<String>,
        cred: &auth_credentials::Credential,
    ) -> anyhow::Result<Self> {
        let access_token = match cred {
            auth_credentials::Credential::ApiKey { key } => key.clone(),
            auth_credentials::Credential::OAuth { access_token, .. } => access_token.clone(),
        };
        let project = std::env::var("GOOGLE_VERTEX_PROJECT")
            .map_err(|e| anyhow::anyhow!("missing GOOGLE_VERTEX_PROJECT: {e}"))?;
        let region =
            std::env::var("GOOGLE_VERTEX_REGION").unwrap_or_else(|_| DEFAULT_REGION.to_string());
        Ok(Self {
            access_token,
            project,
            region,
            model: model.into(),
            max_output_tokens: 4096,
        })
    }

    pub fn with_max_output_tokens(mut self, max: u32) -> Self {
        self.max_output_tokens = max;
        self
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = region.into();
        self
    }

    /// Fully-qualified `streamGenerateContent` endpoint.
    pub fn api_url(&self) -> String {
        format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/google/models/{model}:streamGenerateContent?alt=sse",
            region = self.region,
            project = self.project,
            model = self.model,
        )
    }
}

/// Convert harness `AgentMessage`s into Gemini `contents[]` entries. Identical
/// to the upstream `provider-google` shape.
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

/// Tool definitions wrapped in `functionDeclarations`.
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

/// Stream a response from Vertex AI. Returns an event stream that closes with
/// `done` on success or `error` on failure. Never throws.
pub async fn stream(
    cfg: Arc<VertexConfig>,
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
    cfg: Arc<VertexConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) -> Result<(), VertexError> {
    let url = cfg.api_url();
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
        .header("authorization", format!("Bearer {}", cfg.access_token))
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

    if let Some(usage) = parsed.get("usageMetadata") {
        merge_usage(usage, &mut state.usage);
    }

    if let Some(err) = parsed.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("vertex error");
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

/// Register `provider::google-vertex::complete` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<VertexConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            VertexConfig::with_credential(model, cred)
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

    /// Combined env-var test. Cargo runs unit tests in parallel by default;
    /// folding env-mutating cases into a single test prevents inter-test
    /// races on `GOOGLE_VERTEX_*`.
    #[test]
    fn from_env_behavior_and_url_and_region() {
        let prev_token = std::env::var("GOOGLE_VERTEX_ACCESS_TOKEN").ok();
        let prev_proj = std::env::var("GOOGLE_VERTEX_PROJECT").ok();
        let prev_region = std::env::var("GOOGLE_VERTEX_REGION").ok();

        // from_env_requires_token_and_project
        std::env::remove_var("GOOGLE_VERTEX_ACCESS_TOKEN");
        std::env::remove_var("GOOGLE_VERTEX_PROJECT");
        std::env::remove_var("GOOGLE_VERTEX_REGION");
        assert!(VertexConfig::from_env("gemini-2.5-flash").is_err());

        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "ya29.test");
        assert!(VertexConfig::from_env("gemini-2.5-flash").is_err());

        std::env::set_var("GOOGLE_VERTEX_PROJECT", "my-proj");
        let cfg = VertexConfig::from_env("gemini-2.5-flash").expect("env present");
        assert_eq!(cfg.access_token, "ya29.test");
        assert_eq!(cfg.project, "my-proj");
        assert_eq!(cfg.model, "gemini-2.5-flash");
        assert_eq!(cfg.max_output_tokens, 4096);

        // region_defaults_to_us_central1
        assert_eq!(cfg.region, DEFAULT_REGION);
        assert_eq!(DEFAULT_REGION, "us-central1");

        // api_url_uses_streamGenerateContent_endpoint
        let url = cfg.api_url();
        assert!(url.contains("us-central1-aiplatform.googleapis.com"));
        assert!(url.contains("/projects/my-proj/locations/us-central1/"));
        assert!(url.contains("/publishers/google/models/gemini-2.5-flash:streamGenerateContent"));
        assert!(url.ends_with("?alt=sse"));

        // explicit region wins over default
        std::env::set_var("GOOGLE_VERTEX_REGION", "europe-west4");
        let cfg2 = VertexConfig::from_env("gemini-2.5-flash").expect("env present");
        assert_eq!(cfg2.region, "europe-west4");
        assert!(cfg2
            .api_url()
            .starts_with("https://europe-west4-aiplatform.googleapis.com/"));

        // with_credential reads GOOGLE_VERTEX_PROJECT (+ optional region);
        // ApiKey + OAuth folded together to share the env snapshot.
        let cred = auth_credentials::Credential::ApiKey {
            key: "/path/to/sa.json".into(),
        };
        let cfg_c = VertexConfig::with_credential("gemini-2.5-flash", &cred).unwrap();
        assert_eq!(cfg_c.access_token, "/path/to/sa.json");
        assert_eq!(cfg_c.project, "my-proj");
        assert_eq!(cfg_c.region, "europe-west4");
        assert_eq!(cfg_c.model, "gemini-2.5-flash");

        let cred_oauth = auth_credentials::Credential::OAuth {
            access_token: "ya29.oauth".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            provider_extra: serde_json::Value::Null,
        };
        let cfg_o = VertexConfig::with_credential("gemini-2.5-flash", &cred_oauth).unwrap();
        assert_eq!(cfg_o.access_token, "ya29.oauth");

        // with_credential errors when GOOGLE_VERTEX_PROJECT is missing.
        std::env::remove_var("GOOGLE_VERTEX_PROJECT");
        assert!(VertexConfig::with_credential("gemini-2.5-flash", &cred).is_err());

        match prev_token {
            Some(v) => std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", v),
            None => std::env::remove_var("GOOGLE_VERTEX_ACCESS_TOKEN"),
        }
        match prev_proj {
            Some(v) => std::env::set_var("GOOGLE_VERTEX_PROJECT", v),
            None => std::env::remove_var("GOOGLE_VERTEX_PROJECT"),
        }
        match prev_region {
            Some(v) => std::env::set_var("GOOGLE_VERTEX_REGION", v),
            None => std::env::remove_var("GOOGLE_VERTEX_REGION"),
        }
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
        assert_eq!(wire[0]["functionDeclarations"][0]["name"], "read");
    }
}
