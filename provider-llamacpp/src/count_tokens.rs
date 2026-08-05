//! `provider::llamacpp::count_tokens` — exact prompt token counting by the
//! server that holds the model.
//!
//! `llama-server` counts with the tokenizer baked into the GGUF it loaded, so
//! its answer is not an estimate of the model's tokenizer: it is the model's
//! tokenizer. That matters more here than for any hosted provider, because
//! the operator can load anything — Qwen, Llama, Mistral, a private
//! fine-tune — and no table shipped in this binary could know which
//! vocabulary is in memory right now.
//!
//! The route used is llama.cpp's Anthropic-compatible
//! `POST /v1/messages/count_tokens`, which takes messages and answers
//! `{"input_tokens": N}`. Counting is local to the operator's machine and
//! costs nothing.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::endpoint_count::{post_count, ESTIMATOR_METERED};
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
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

/// Derive the counting endpoint — a server-root path like `/props`, not a
/// child of the chat route (`…/v1/chat/completions` →
/// `…/v1/messages/count_tokens`).
fn count_tokens_url(api_url: &str) -> String {
    match api_url.strip_suffix("/v1/chat/completions") {
        Some(root) => format!("{root}/v1/messages/count_tokens"),
        None => format!("{}/v1/messages/count_tokens", api_url.trim_end_matches('/')),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets. `llama-server` serves one model at a
    /// time and counts with whichever it loaded, so this is carried for the
    /// reply rather than to select a tokenizer.
    pub model: String,
    /// System prompt counted as the leading wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; mapped to the wire `tools` array.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::llamacpp::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens the server counted with the loaded model's tokenizer.
    pub tokens: u64,
    /// Always `metered`: the server that holds the model produced the count.
    pub estimator: String,
}

fn build_count_body(
    model: &str,
    system_prompt: &str,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": to_wire_messages(messages, system_prompt),
    });
    if !tools.is_empty() {
        body["tools"] = json!(functions_to_wire(tools));
    }
    body
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

    let body = build_count_body(
        &cfg.model,
        req.system_prompt.as_deref().unwrap_or(""),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    let headers = build_headers(&cfg)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();

    let tokens = post_count(
        http,
        count_tokens_url(&cfg.api_url),
        headers,
        body,
        |reply| reply.get("input_tokens").and_then(Value::as_u64),
    )
    .await?;

    Ok(CountTokensResponse {
        model: cfg.model,
        tokens,
        estimator: ESTIMATOR_METERED.into(),
    })
}
