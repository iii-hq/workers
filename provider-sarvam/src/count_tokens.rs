//! `provider::sarvam::count_tokens` — local prompt token estimation with the
//! tiktoken tokenizers. Sarvam exposes no metering endpoint and publishes
//! no tokenizer for the 105B and 30B models, so the count is an estimate
//! from the text the wire mappers would send plus the chat-framing
//! constants; it never runs the model, costs nothing, and needs no network.
//! The counting logic is shared with the other OpenAI-shaped providers in
//! `llm_router::provider_scaffold::tiktoken_count`.
use iii_sdk::errors::Error;
use llm_router::provider_scaffold::tiktoken_count::{count_chat_tokens, ESTIMATOR_TIKTOKEN};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; reported back unchanged.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward
    /// the total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::sarvam::stream`
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

    #[test]
    fn empty_messages_are_rejected() {
        let req = CountTokensRequest {
            model: "sarvam-105b".into(),
            system_prompt: None,
            tools: None,
            messages: vec![],
        };
        assert!(handle(req).is_err());
    }

    #[test]
    fn handle_wraps_the_scaffold_estimate() {
        let req = CountTokensRequest {
            model: "sarvam-105b".into(),
            system_prompt: None,
            tools: None,
            messages: vec![user("namaste duniya")],
        };
        let resp = handle(req).unwrap();
        let expected = count_chat_tokens("sarvam-105b", None, &[], &[user("namaste duniya")]);
        assert_eq!(resp.tokens, expected);
        assert_eq!(resp.estimator, "tiktoken");
    }
}
