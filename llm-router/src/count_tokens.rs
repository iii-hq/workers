//! `router::count_tokens` — one front door for prompt token counting,
//! resolved with the SAME `decide()` the chat pipeline uses: `{model,
//! provider?}` maps to a provider, and the request is forwarded to
//! `provider::<id>::count_tokens`. Providers answer with an exact count from
//! their metering API (`estimator: "provider"`) or a local tokenizer
//! estimate (`estimator: "tiktoken"`); a provider without the surface is a
//! typed `router/no_token_counter` error so callers can fall back to their
//! own estimate. Counting never runs the model and costs nothing.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::store::CatalogStore;
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::store::RegistryStore;
use crate::routing::{decide, DecideInput};
use crate::types::errors::{is_function_not_found, RouterCode, RouterError};
use crate::types::messages::AgentMessage;
use crate::types::model::AgentFunction;

/// Counting is bounded and non-streaming, like `router::embed`.
const COUNT_TOKENS_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RouterCountTokensRequest {
    /// Model id the prompt targets; routes exactly like `router::chat` and
    /// selects the provider's tokenizer. Optional only when `provider` pins
    /// the destination.
    #[serde(default)]
    pub model: Option<String>,
    /// Pin an explicit provider, bypassing heuristics (optional).
    #[serde(default)]
    pub provider: Option<String>,
    /// System prompt counted as part of the request (optional).
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas the turn would carry; their serialized
    /// schemas count toward the total (optional).
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `router::chat` accepts. Must be
    /// non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RouterCountTokensResponse {
    pub provider: String,
    pub model: String,
    /// Prompt tokens the provider counted for the assembled request.
    pub tokens: u64,
    /// `provider` when a provider metering API produced the count,
    /// `tiktoken` when a local tokenizer estimated it.
    pub estimator: String,
}

/// What `provider::<id>::count_tokens` answers; `provider` is stamped on by
/// the router.
#[derive(Debug, Deserialize)]
struct ProviderCountTokensReply {
    model: String,
    tokens: u64,
    estimator: String,
}

pub fn make_count_tokens(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    config: ConfigCell,
) -> impl Fn(RouterCountTokensRequest) -> BoxedCountTokensFuture + Send + Sync + 'static {
    move |req: RouterCountTokensRequest| {
        let (iii, registry, catalog, config) = (
            iii.clone(),
            registry.clone(),
            catalog.clone(),
            config.clone(),
        );
        Box::pin(async move {
            // Providers stay dumb pipes: the router never injects placeholder
            // messages to make an empty request countable.
            if req.messages.is_empty() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "messages must not be empty",
                )
                .into());
            }
            let model = req.model.unwrap_or_default();
            if model.is_empty() && req.provider.is_none() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "model or provider is required",
                )
                .into());
            }
            // Same inputs, same decide(), same error codes as the chat
            // pipeline's routing step.
            let config = snapshot(&config);
            let heuristics = config.settings().routing_heuristics.clone();
            let default_provider = config.settings().default_provider.clone();
            let candidates = decide(&DecideInput {
                model: model.clone(),
                provider: req.provider,
                registered_providers: registry.ids().await,
                catalog: catalog.model_ids().await,
                heuristics,
                default_provider,
            })
            .map_err(Error::from)?;
            let provider = candidates[0].clone();

            let reply = iii
                .trigger(TriggerRequest {
                    function_id: format!("provider::{provider}::count_tokens"),
                    payload: json!({
                        "model": model,
                        "system_prompt": req.system_prompt,
                        "tools": req.tools,
                        "messages": req.messages,
                    }),
                    action: None,
                    timeout_ms: Some(COUNT_TOKENS_TIMEOUT_MS),
                })
                .await
                .map_err(|e| {
                    if is_function_not_found(&e) {
                        Error::Handler(format!(
                            "router/no_token_counter: provider '{provider}' has no token counter"
                        ))
                    } else {
                        e
                    }
                })?;
            let reply: ProviderCountTokensReply = serde_json::from_value(reply).map_err(|e| {
                Error::Handler(format!(
                    "router/bad_provider_response: provider::{provider}::count_tokens \
                         returned an invalid response: {e}"
                ))
            })?;
            Ok(RouterCountTokensResponse {
                provider,
                model: reply.model,
                tokens: reply.tokens,
                estimator: reply.estimator,
            })
        })
    }
}

type BoxedCountTokensFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterCountTokensResponse, Error>> + Send>>;
