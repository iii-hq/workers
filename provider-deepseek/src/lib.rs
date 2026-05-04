//! DeepSeek Chat Completions streaming via provider-base.

use std::sync::Arc;

use harness_types::{AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, StopReason};
use provider_base::{stream_chat_completions, ChatCompletionsConfig, OpenAICompatRequest};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

const API_URL: &str = "https://api.deepseek.com/v1/chat/completions";
const PROVIDER_NAME: &str = "deepseek";
const ENV_VAR: &str = "DEEPSEEK_API_KEY";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeepSeekConfig {
    /// Header-bearer credential. Accepts either an API key or an OAuth
    /// access token — both are sent verbatim as `Authorization: Bearer
    /// <value>`. Populated via [`DeepSeekConfig::with_credential`] in
    /// P5; the legacy [`DeepSeekConfig::from_env`] constructor still
    /// reads the env var directly.
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

impl DeepSeekConfig {
    pub fn from_env(model: impl Into<String>) -> Result<Self, std::env::VarError> {
        let api_key = std::env::var(ENV_VAR)?;
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
    cfg: Arc<DeepSeekConfig>,
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

/// Register `provider::deepseek::stream` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<DeepSeekConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            DeepSeekConfig::with_credential(model, cred)
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

    // Single test guards against env-var races between parallel test threads.
    #[test]
    fn config_from_env_behavior() {
        let prev = std::env::var(ENV_VAR).ok();
        std::env::remove_var(ENV_VAR);
        assert!(DeepSeekConfig::from_env("test-model").is_err());
        std::env::set_var(ENV_VAR, "test-key");
        let cfg = DeepSeekConfig::from_env("test-model").unwrap();
        assert_eq!(cfg.api_key, "test-key");
        assert_eq!(cfg.model, "test-model");
        assert_eq!(cfg.max_tokens, 4096);
        match prev {
            Some(v) => std::env::set_var(ENV_VAR, v),
            None => std::env::remove_var(ENV_VAR),
        }
    }

    #[test]
    fn with_credential_api_key() {
        let cred = auth_credentials::Credential::ApiKey {
            key: "sk-test".into(),
        };
        let cfg = DeepSeekConfig::with_credential("the-model", &cred).unwrap();
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
        let cfg = DeepSeekConfig::with_credential("the-model", &cred).unwrap();
        assert_eq!(cfg.api_key, "tok");
    }
}
