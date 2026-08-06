//! `provider::xai::count_tokens` — prompt token counting with Grok's own
//! tokenizer, reached over the wire.
//!
//! xAI publishes no endpoint that meters a whole prompt; what it publishes is
//! a tokenizer (`…/tokenize-text`), which takes text and answers with the
//! tokens it becomes. So the split here is unlike the other providers: xAI
//! owns the vocabulary, this worker owns the chat framing. The request is
//! reduced to its counted text by the shared framing rules, tokenized in one
//! call rather than one per row, and the framing is added back on top.
//!
//! Two consequences worth knowing. The join costs a separator token between
//! rows, so the count runs a few tokens high on a request with many messages.
//! And xAI's own FAQ notes this tokenizer can disagree with what billing
//! records, so this is Grok's tokenizer rather than Grok's invoice.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::chat_framing::frame;
use llm_router::provider_scaffold::endpoint_count::{base_route_url, post_count};
use llm_router::provider_scaffold::vocabulary_count::ESTIMATOR_TOKENIZER;
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::build_headers;
use crate::state;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; selects the tokenizer upstream.
    pub model: String,
    /// System prompt counted as its own framed wire row when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::xai::stream` accepts.
    /// Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request: Grok's tokenizer over the
    /// text, this worker's framing on top.
    pub tokens: u64,
    /// Always `tokenizer`: a real vocabulary produced the count, but the
    /// upstream metered the text rather than the request.
    pub estimator: String,
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }

    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(
            iii,
            crate::PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            return Err(e);
        }
    };
    let cfg = config_from_resolve(&req.model, None, &resolved)
        .map_err(|e| Error::Handler(e.to_string()))?;

    let framed = frame(
        req.system_prompt.as_deref(),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    let headers = build_headers(&cfg)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();

    let text_tokens = post_count(
        http,
        base_route_url(&cfg.api_url, "tokenize-text"),
        headers,
        json!({ "model": cfg.model, "text": framed.joined() }),
        // The reply is the tokens themselves, so the count is their number.
        |reply| {
            reply
                .get("token_ids")
                .and_then(Value::as_array)
                .map(|tokens| tokens.len() as u64)
        },
    )
    .await?;

    Ok(CountTokensResponse {
        model: cfg.model,
        tokens: framed.total_from(text_tokens),
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}
