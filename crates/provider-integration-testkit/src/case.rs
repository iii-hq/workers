use std::future::Future;
use std::pin::Pin;

use iii_sdk::IIIClient;

pub const ANTHROPIC_MESSAGES: &str = "anthropic-messages";
pub const OPENAI_CHAT_COMPLETIONS: &str = "openai-chat-completions";
pub const OPENAI_RESPONSES: &str = "openai-responses";

type RegisterFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
type RegisterProvider = fn(IIIClient) -> RegisterFuture;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolFamily {
    AnthropicMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
}

impl ProtocolFamily {
    pub fn id(self) -> &'static str {
        match self {
            Self::AnthropicMessages => ANTHROPIC_MESSAGES,
            Self::OpenAiChatCompletions => OPENAI_CHAT_COMPLETIONS,
            Self::OpenAiResponses => OPENAI_RESPONSES,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) enum CredentialMode {
    ApiKey,
    ClaudeOauth,
    CodexOauth,
}

#[derive(Clone, Copy)]
pub(crate) struct ProviderCase {
    pub(crate) id: &'static str,
    pub(crate) family: ProtocolFamily,
    pub(crate) model: &'static str,
    pub(crate) alternate_model: &'static str,
    pub(crate) upstream_model: &'static str,
    pub(crate) alternate_upstream_model: &'static str,
    pub(crate) generation_path: &'static str,
    pub(crate) credential: CredentialMode,
    pub(crate) register: RegisterProvider,
}

impl std::fmt::Debug for ProviderCase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderCase")
            .field("id", &self.id)
            .field("family", &self.family)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

pub(crate) fn enabled_cases() -> Vec<ProviderCase> {
    let mut cases = Vec::new();
    #[cfg(feature = "provider-anthropic")]
    cases.push(ProviderCase {
        id: "anthropic",
        family: ProtocolFamily::AnthropicMessages,
        model: "claude-sonnet-4-6",
        alternate_model: "claude-opus-4-8",
        upstream_model: "claude-sonnet-4-6",
        alternate_upstream_model: "claude-opus-4-8",
        generation_path: "/v1/messages",
        credential: CredentialMode::ApiKey,
        register: |iii| {
            Box::pin(async move {
                provider_anthropic::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    });
    #[cfg(feature = "provider-claude-code")]
    cases.push(ProviderCase {
        id: "claude-code",
        family: ProtocolFamily::AnthropicMessages,
        model: "claude-code/claude-sonnet-4-6",
        alternate_model: "claude-code/claude-opus-4-8",
        upstream_model: "claude-sonnet-4-6",
        alternate_upstream_model: "claude-opus-4-8",
        generation_path: "/v1/messages",
        credential: CredentialMode::ClaudeOauth,
        register: |iii| {
            Box::pin(async move {
                provider_claude_code::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    });
    #[cfg(feature = "provider-command-code")]
    {
        cases.push(ProviderCase {
            id: "command-code",
            family: ProtocolFamily::OpenAiChatCompletions,
            model: "command-code/gpt-5.4",
            alternate_model: "command-code/gpt-5.6-luna",
            upstream_model: "gpt-5.4",
            alternate_upstream_model: "gpt-5.6-luna",
            generation_path: "/chat/completions",
            credential: CredentialMode::ApiKey,
            register: |iii| {
                Box::pin(async move {
                    provider_command_code::register::register_provider(iii)
                        .await
                        .map_err(anyhow::Error::from)
                })
            },
        });
        cases.push(ProviderCase {
            id: "command-code",
            family: ProtocolFamily::AnthropicMessages,
            model: "command-code/claude-sonnet-4-6",
            alternate_model: "command-code/claude-opus-4-8",
            upstream_model: "claude-sonnet-4-6",
            alternate_upstream_model: "claude-opus-4-8",
            generation_path: "/messages",
            credential: CredentialMode::ApiKey,
            register: |iii| {
                Box::pin(async move {
                    provider_command_code::register::register_provider(iii)
                        .await
                        .map_err(anyhow::Error::from)
                })
            },
        });
    }
    #[cfg(feature = "provider-deepseek")]
    cases.push(openai_chat_case(
        "deepseek",
        "deepseek-v4-pro",
        "deepseek-v4-flash",
        "/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_deepseek::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-kimi")]
    cases.push(openai_chat_case(
        "kimi",
        "kimi-k2-0905-preview",
        "kimi-k2-thinking",
        "/v1/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_kimi::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-openai")]
    {
        cases.push(ProviderCase {
            id: "openai",
            family: ProtocolFamily::OpenAiResponses,
            model: "gpt-5.2",
            alternate_model: "gpt-5.6-luna",
            upstream_model: "gpt-5.2",
            alternate_upstream_model: "gpt-5.6-luna",
            generation_path: "/v1/responses",
            credential: CredentialMode::ApiKey,
            register: |iii| {
                Box::pin(async move {
                    provider_openai::register::register_provider(iii)
                        .await
                        .map_err(anyhow::Error::from)
                })
            },
        });
        cases.push(openai_chat_case(
            "openai",
            "gpt-5.2",
            "gpt-5.6-luna",
            "/v1/chat/completions",
            |iii| {
                Box::pin(async move {
                    provider_openai::register::register_provider(iii)
                        .await
                        .map_err(anyhow::Error::from)
                })
            },
        ));
    }
    #[cfg(feature = "provider-openai-codex")]
    cases.push(ProviderCase {
        id: "openai-codex",
        family: ProtocolFamily::OpenAiResponses,
        model: "codex/gpt-5.2",
        alternate_model: "codex/gpt-5.6-luna",
        upstream_model: "gpt-5.2",
        alternate_upstream_model: "gpt-5.6-luna",
        generation_path: "/backend-api/codex/responses",
        credential: CredentialMode::CodexOauth,
        register: |iii| {
            Box::pin(async move {
                provider_openai_codex::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    });
    #[cfg(feature = "provider-openrouter")]
    cases.push(openai_chat_case(
        "openrouter",
        "openrouter/vendor-a/agentic",
        "openrouter/vendor-b/reasoning",
        "/api/v1/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_openrouter::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-xai")]
    cases.push(openai_chat_case(
        "xai",
        "grok-4",
        "grok-4-fast",
        "/v1/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_xai::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-zai")]
    cases.push(openai_chat_case(
        "zai",
        "glm-4.7",
        "glm-5",
        "/api/coding/paas/v4/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_zai::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    cases
}

#[allow(dead_code)]
fn openai_chat_case(
    id: &'static str,
    model: &'static str,
    alternate_model: &'static str,
    generation_path: &'static str,
    register: RegisterProvider,
) -> ProviderCase {
    let upstream_model = model.strip_prefix("openrouter/").unwrap_or(model);
    let alternate_upstream_model = alternate_model
        .strip_prefix("openrouter/")
        .unwrap_or(alternate_model);
    ProviderCase {
        id,
        family: ProtocolFamily::OpenAiChatCompletions,
        model,
        alternate_model,
        upstream_model,
        alternate_upstream_model,
        generation_path,
        credential: CredentialMode::ApiKey,
        register,
    }
}
