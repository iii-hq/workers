//! Streaming client for the OpenAI Chat Completions API.
//!
//! Implements the `StreamFn` contract used by the harness loop: never throws,
//! always returns an event-yielding stream that ends with `done` or `error`.
//!
//! This crate is the trivial OpenAI-compat case — it wraps
//! [`provider_base::stream_chat_completions`] with a fixed endpoint, provider
//! name, and `Authorization: Bearer` header. For richer wire shapes (Responses
//! API, reasoning items, tool-call streaming events) see
//! `provider-openai-responses`.

use std::sync::Arc;

use harness_types::{
    AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind,
    StopReason,
};
use provider_base::{stream_chat_completions, ChatCompletionsConfig, OpenAICompatRequest};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Default Chat Completions endpoint.
pub const DEFAULT_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Provider name reported on every emitted `AssistantMessage`.
pub const PROVIDER_NAME: &str = "openai";

/// Configuration for a single OpenAI Chat Completions streaming call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenAIConfig {
    /// Header-bearer credential. Accepts either an API key or an OAuth
    /// access token — both are sent verbatim as `Authorization: Bearer
    /// <value>`. Populated via [`OpenAIConfig::with_credential`] in P5;
    /// the legacy [`OpenAIConfig::from_env`] constructor still reads the
    /// env var directly.
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub api_url: String,
}

impl OpenAIConfig {
    /// Build a config from `OPENAI_API_KEY`. Defaults `max_tokens` to 4096 and
    /// `api_url` to [`DEFAULT_API_URL`].
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let key = std::env::var("OPENAI_API_KEY")?;
        Ok(Self {
            api_key: key,
            model: model.into(),
            max_tokens: 4096,
            api_url: DEFAULT_API_URL.into(),
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// Both `Credential::ApiKey` and `Credential::OAuth` collapse into the
    /// same Bearer header, so no `AuthMode` is needed (unlike Anthropic).
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
            max_tokens: 4096,
            api_url: DEFAULT_API_URL.into(),
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

/// Stream a response from OpenAI Chat Completions. Returns an event stream
/// that closes with `done` on success or `error` on failure. Never throws.
pub async fn stream(
    cfg: Arc<OpenAIConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
) -> ReceiverStream<AssistantMessageEvent> {
    let base_cfg = Arc::new(
        ChatCompletionsConfig::new(
            cfg.api_url.clone(),
            PROVIDER_NAME,
            cfg.model.clone(),
            cfg.api_key.clone(),
        )
        .with_max_tokens(cfg.max_tokens),
    );
    stream_chat_completions(
        base_cfg,
        OpenAICompatRequest {
            system_prompt,
            messages,
            tools,
        },
    )
    .await
}

/// Register `provider::openai::stream` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<OpenAIConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            OpenAIConfig::with_credential(model, cred)
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
        content: vec![ContentBlock::Text(harness_types::TextContent {
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

    /// Combined env-mutating test. Cargo runs tests in parallel within a
    /// crate; splitting these into two tests creates a race on the shared
    /// `OPENAI_API_KEY` env var. Coverage is identical.
    #[test]
    fn from_env_reads_or_errors_per_state() {
        let prev = std::env::var("OPENAI_API_KEY").ok();

        std::env::remove_var("OPENAI_API_KEY");
        assert!(OpenAIConfig::from_env("gpt-4o-mini").is_err());

        std::env::set_var("OPENAI_API_KEY", "sk-test-fixture");
        let cfg = OpenAIConfig::from_env("gpt-4o-mini").expect("env present");
        assert_eq!(cfg.api_key, "sk-test-fixture");
        assert_eq!(cfg.model, "gpt-4o-mini");
        assert_eq!(cfg.max_tokens, 4096);
        assert_eq!(cfg.api_url, DEFAULT_API_URL);

        match prev {
            Some(v) => std::env::set_var("OPENAI_API_KEY", v),
            None => std::env::remove_var("OPENAI_API_KEY"),
        }
    }

    #[test]
    fn builder_overrides_apply() {
        let cfg = OpenAIConfig {
            api_key: "k".into(),
            model: "gpt-4o".into(),
            max_tokens: 4096,
            api_url: DEFAULT_API_URL.into(),
        }
        .with_max_tokens(8192)
        .with_api_url("https://example.test/v1/chat/completions");
        assert_eq!(cfg.max_tokens, 8192);
        assert_eq!(cfg.api_url, "https://example.test/v1/chat/completions");
    }

    #[test]
    fn with_credential_api_key() {
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-test".into(),
        };
        let cfg = OpenAIConfig::with_credential("the-model", &cred).unwrap();
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
        let cfg = OpenAIConfig::with_credential("the-model", &cred).unwrap();
        assert_eq!(cfg.api_key, "tok");
    }

    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY"]
    async fn live_stream_smoke() {
        if std::env::var("OPENAI_API_KEY").is_err() {
            return;
        }
        let cfg = Arc::new(OpenAIConfig::from_env("gpt-4o-mini").unwrap());
        let s = stream(cfg, "You are terse.".into(), Vec::new(), Vec::new()).await;
        let msg = collect(s).await;
        assert_eq!(msg.provider, PROVIDER_NAME);
    }
}
