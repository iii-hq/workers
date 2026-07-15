//! `router::embed` — one front door for text embeddings, mirroring the
//! chat surface's shape: callers name a provider or let the router find
//! one. Providers implement `provider::<id>::embed` (batch in, one vector
//! per input, order preserved); the router discovers implementations from
//! the live function registry, so a new provider needs zero router
//! changes to serve embeddings.

use std::future::Future;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const EMBED_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RouterEmbedRequest {
    /// Embedding model id; the provider's default when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Provider id (e.g. `openai`); the first embed-capable provider in
    /// the live registry when omitted.
    #[serde(default)]
    pub provider: Option<String>,
    /// Texts to embed, one vector returned per input, order preserved.
    pub input: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RouterEmbedResponse {
    pub provider: String,
    pub model: String,
    /// One embedding per input, in input order.
    pub embeddings: Vec<Vec<f32>>,
}

/// Embed-capable provider ids from the live function registry
/// (`provider::<id>::embed`), stable-sorted with `openai` preferred so the
/// default choice is deterministic.
async fn discover_providers(iii: &IIIClient) -> Vec<String> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({ "search": "embed" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    let mut ids: Vec<String> = reply
        .ok()
        .and_then(|v| v.get("functions").cloned())
        .and_then(|f| serde_json::from_value::<Vec<Value>>(f).ok())
        .map(|fns| {
            fns.iter()
                .filter_map(|f| f.get("function_id").and_then(Value::as_str))
                .filter_map(|id| {
                    id.strip_prefix("provider::")
                        .and_then(|rest| rest.strip_suffix("::embed"))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    if let Some(pos) = ids.iter().position(|p| p == "openai") {
        ids.swap(0, pos);
    }
    ids
}

pub fn make_embed(
    iii: IIIClient,
) -> impl Fn(RouterEmbedRequest) -> BoxedEmbedFuture + Send + Sync + 'static {
    move |req: RouterEmbedRequest| {
        let iii = iii.clone();
        Box::pin(async move {
            if req.input.is_empty() {
                return Err(Error::Handler(
                    "router/invalid_input: input must not be empty".into(),
                ));
            }
            let provider = match req.provider.filter(|p| !p.trim().is_empty()) {
                Some(p) => p,
                None => discover_providers(&iii)
                    .await
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        Error::Handler(
                            "router/no_embed_provider: no provider::<id>::embed function is \
                             registered; add a provider worker with an embeddings surface"
                                .into(),
                        )
                    })?,
            };
            let reply = iii
                .trigger(TriggerRequest {
                    function_id: format!("provider::{provider}::embed"),
                    payload: json!({ "model": req.model, "input": req.input }),
                    action: None,
                    timeout_ms: Some(EMBED_TIMEOUT_MS),
                })
                .await?;
            let model = reply
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let embeddings: Vec<Vec<f32>> = reply
                .get("embeddings")
                .cloned()
                .and_then(|e| serde_json::from_value(e).ok())
                .ok_or_else(|| {
                    Error::Handler(format!(
                        "router/bad_provider_response: provider::{provider}::embed returned no embeddings array"
                    ))
                })?;
            Ok(RouterEmbedResponse {
                provider,
                model,
                embeddings,
            })
        })
    }
}

type BoxedEmbedFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterEmbedResponse, Error>> + Send>>;
