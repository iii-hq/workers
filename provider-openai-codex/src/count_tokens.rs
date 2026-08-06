//! `provider::openai-codex::count_tokens` — local prompt token estimation
//! with the tiktoken tokenizers (vocabularies embedded in the binary). The
//! Codex backend exposes no metering endpoint, so the count is computed from
//! the text the wire mappers would send plus the published chat-framing
//! constants; it never runs the model, costs nothing, and needs no network.
//! The counting logic itself is shared with `provider-openai` in
//! `llm_router::provider_scaffold::tiktoken_count`; this module is the thin
//! request/response adapter around it. Model ids stay namespaced
//! (`codex/gpt-5.2`) end to end — the shared encoder-selection logic strips
//! the namespace internally, but the response echoes the id the caller sent.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::tiktoken_count::{count_chat_tokens, ESTIMATOR_TIKTOKEN};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; selects the tokenizer (o200k_base for
    /// gpt-4o/gpt-5/o-series and anything unknown-modern, cl100k_base for
    /// gpt-3.5 and non-o gpt-4 families).
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward
    /// the total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::openai-codex::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Estimated prompt tokens for the assembled request.
    pub tokens: u64,
    /// Always `tiktoken`: a local tokenizer produced the estimate.
    pub estimator: String,
}

pub fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let tokens = count_chat_tokens(
        &req.model,
        req.system_prompt.as_deref(),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TIKTOKEN.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    fn request(model: &str, messages: Vec<AgentMessage>) -> CountTokensRequest {
        CountTokensRequest {
            model: model.into(),
            system_prompt: None,
            tools: None,
            messages,
        }
    }

    #[test]
    fn empty_messages_are_rejected() {
        assert!(handle(request("codex/gpt-5.2", vec![])).is_err());
    }

    #[test]
    fn handle_strips_the_codex_namespace_for_encoding_but_echoes_the_model_id() {
        let namespaced = handle(request("codex/gpt-5.2", vec![user("hi")])).unwrap();
        let bare = handle(request("gpt-5.2", vec![user("hi")])).unwrap();
        assert_eq!(namespaced.tokens, bare.tokens);
        assert_eq!(namespaced.model, "codex/gpt-5.2");
        assert_eq!(namespaced.estimator, "tiktoken");
    }
}
