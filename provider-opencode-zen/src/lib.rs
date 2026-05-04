//! OpenCode Zen Chat Completions streaming via provider-base.

use std::sync::Arc;

use harness_types::{AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, StopReason};
use provider_base::{stream_chat_completions, ChatCompletionsConfig, OpenAICompatRequest};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

// TODO: confirm endpoint with provider docs
const API_URL: &str = "https://api.opencode.ai/v1/chat/completions";
const PROVIDER_NAME: &str = "opencode-zen";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpencodeZenConfig {
    /// Header-bearer credential. Accepts either an API key or an OAuth
    /// access token — both are sent verbatim as `Authorization: Bearer
    /// <value>`. Populated via [`OpencodeZenConfig::with_credential`] in
    /// P5; the legacy [`OpencodeZenConfig::from_env`] constructor still
    /// reads the env var directly.
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

impl OpencodeZenConfig {
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let api_key = std::env::var("OPENCODE_ZEN_API_KEY")?;
        Ok(Self {
            api_key,
            model: model.into(),
            max_tokens: 4096,
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
        })
    }
}

pub async fn stream(
    cfg: Arc<OpencodeZenConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
) -> ReceiverStream<AssistantMessageEvent> {
    let base = ChatCompletionsConfig::new(
        API_URL,
        PROVIDER_NAME,
        cfg.model.clone(),
        cfg.api_key.clone(),
    )
    .with_max_tokens(cfg.max_tokens);
    let req = OpenAICompatRequest {
        system_prompt,
        messages,
        tools,
    };
    stream_chat_completions(Arc::new(base), req).await
}

/// Register `provider::opencode-zen::stream` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<OpencodeZenConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            OpencodeZenConfig::with_credential(model, cred)
        },
        stream,
    );
    Ok(())
}

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
            | AssistantMessageEvent::ThinkingEnd { partial } => last = Some(partial),
            _ => {}
        }
    }
    last.unwrap_or_else(|| AssistantMessage {
        content: vec![],
        stop_reason: StopReason::Error,
        error_message: Some("stream closed without final".into()),
        error_kind: Some(harness_types::ErrorKind::Transient),
        usage: None,
        model: "unknown".into(),
        provider: PROVIDER_NAME.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Combined env-mutating test. Cargo runs tests in parallel within a crate,
    /// so splitting these caused a race on `OPENCODE_ZEN_API_KEY`. Coverage is identical
    /// when sequenced.
    #[test]
    fn config_env_resolution() {
        let prev = std::env::var("OPENCODE_ZEN_API_KEY").ok();

        std::env::remove_var("OPENCODE_ZEN_API_KEY");
        assert!(OpencodeZenConfig::from_env("test-model").is_err());

        std::env::set_var("OPENCODE_ZEN_API_KEY", "test-key");
        let cfg = OpencodeZenConfig::from_env("test-model").expect("ok");
        assert_eq!(cfg.model, "test-model");
        assert_eq!(cfg.max_tokens, 4096);

        match prev {
            Some(v) => std::env::set_var("OPENCODE_ZEN_API_KEY", v),
            None => std::env::remove_var("OPENCODE_ZEN_API_KEY"),
        }
    }

    #[test]
    fn with_credential_api_key() {
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-test".into(),
        };
        let cfg = OpencodeZenConfig::with_credential("the-model", &cred).unwrap();
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
        let cfg = OpencodeZenConfig::with_credential("the-model", &cred).unwrap();
        assert_eq!(cfg.api_key, "tok");
    }
}
