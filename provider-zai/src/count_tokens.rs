//! `provider::zai::count_tokens` — prompt token counting with GLM's own
//! vocabulary.
//!
//! Z.AI publishes no metering endpoint, so the count is computed locally, and
//! computed with the vocabulary GLM was trained with rather than a borrowed
//! one. The OpenAI-compatible wire shape does not imply an OpenAI tokenizer;
//! counting these models with tiktoken would produce a number that reads as
//! authoritative while being wrong by whatever the two vocabularies disagree
//! about — and GLM's disagrees most on exactly the Chinese text these models
//! are used for.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::vocabulary_count::{
    count_chat_tokens, resolve, VocabularyRef, ESTIMATOR_TOKENIZER,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The GLM chat family shares one vocabulary, so the reference is fixed
/// rather than per-model: the id list turns over far faster than the
/// tokenizer behind it.
fn vocabulary() -> VocabularyRef {
    VocabularyRef::huggingface("zai-org/GLM-4.6")
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets. Reported back unchanged; the GLM chat
    /// models share the one vocabulary.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::zai::stream` accepts.
    /// Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request, by GLM's vocabulary.
    pub tokens: u64,
    /// Always `tokenizer`: the model's own vocabulary produced the count.
    pub estimator: String,
}

pub async fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let tokenizer = resolve(&vocabulary()).await.ok_or_else(|| {
        Error::Handler(
            "router/no_token_counter: GLM's vocabulary is not cached and could not \
             be fetched"
                .into(),
        )
    })?;
    let tokens = count_chat_tokens(
        &tokenizer,
        req.system_prompt.as_deref(),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}
