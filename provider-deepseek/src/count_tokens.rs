//! `provider::deepseek::count_tokens` — prompt token counting with DeepSeek's
//! own vocabulary.
//!
//! DeepSeek publishes no metering endpoint, so the count is computed locally.
//! It is computed with DeepSeek's published tokenizer rather than a borrowed
//! one: the OpenAI-compatible wire shape does not imply an OpenAI vocabulary,
//! and counting these models with tiktoken would produce a number that looks
//! authoritative while being wrong by whatever the two vocabularies disagree
//! about. The vocabulary is fetched once and cached on disk, so a model
//! DeepSeek ships tomorrow counts correctly today.
//!
//! What gets counted is the prompt DeepSeek builds from this provider's wire
//! rows, not the rows' text: the tool block, the DSML each call becomes and
//! the replayed `reasoning_content` are billed as prompt tokens too. Counting
//! the text alone left `context_pressure` ~7% under its bill.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::deepseek_prompt::{prompt, vocabulary};
use llm_router::provider_scaffold::vocabulary_count::{resolve, ESTIMATOR_TOKENIZER};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets. Reported back unchanged; every DeepSeek
    /// chat model shares the one vocabulary.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::deepseek::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request, by DeepSeek's vocabulary.
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
    // A cold cache with no network is the one case with no honest answer
    // here. Reporting the typed no_token_counter error leaves the caller on
    // its own estimate, which is the same place it would have been without
    // this function — better than a number computed from the wrong
    // vocabulary.
    let tokenizer = resolve(&vocabulary()).await.ok_or_else(|| {
        Error::Handler(
            "router/no_token_counter: DeepSeek's vocabulary is not cached and could \
             not be fetched"
                .into(),
        )
    })?;
    let rows = to_wire_messages(
        &req.messages,
        req.system_prompt.as_deref().unwrap_or_default(),
    );
    let tools = functions_to_wire(req.tools.as_deref().unwrap_or(&[]));
    let tokens = tokenizer
        .encode(prompt(&rows, &tools), false)
        .map_err(|err| Error::Handler(format!("router/no_token_counter: {err}")))?
        .len() as u64;
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}
