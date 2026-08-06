//! `router::embed` — one front door for text embeddings, mirroring the
//! chat surface's shape: callers name a provider or let the router pick
//! one from its own provider registry. Providers implement
//! `provider::<id>::embed` (batch in, one vector per input, order
//! preserved); registered providers without the surface are skipped on a
//! function-not-found, so a new embed-capable provider needs zero router
//! changes to serve embeddings.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::registry::store::RegistryStore;
use crate::types::errors::is_function_not_found;

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

/// One provider attempt. `Ok(None)` = this provider has no embed surface
/// (function not found) — try the next.
async fn try_provider(
    iii: &IIIClient,
    provider: &str,
    model: &Option<String>,
    input: &[String],
) -> Result<Option<RouterEmbedResponse>, Error> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: format!("provider::{provider}::embed"),
            payload: json!({ "model": model, "input": input }),
            action: None,
            timeout_ms: Some(EMBED_TIMEOUT_MS),
        })
        .await;
    let reply = match reply {
        Ok(v) => v,
        Err(e) if is_function_not_found(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let model = reply
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            Error::Handler(format!(
                "router/bad_provider_response: provider::{provider}::embed returned no model"
            ))
        })?;
    let embeddings: Vec<Vec<f32>> = reply
        .get("embeddings")
        .cloned()
        .and_then(|e| serde_json::from_value(e).ok())
        .ok_or_else(|| {
            Error::Handler(format!(
                "router/bad_provider_response: provider::{provider}::embed returned no embeddings array"
            ))
        })?;
    // Callers zip vectors back onto their inputs by position; a count
    // mismatch would misattribute every vector after the gap.
    if embeddings.len() != input.len() {
        return Err(Error::Handler(format!(
            "router/bad_provider_response: provider::{provider}::embed returned {} embeddings for {} inputs",
            embeddings.len(),
            input.len()
        )));
    }
    Ok(Some(RouterEmbedResponse {
        provider: provider.to_string(),
        model,
        embeddings,
    }))
}

pub fn make_embed(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
) -> impl Fn(RouterEmbedRequest) -> BoxedEmbedFuture + Send + Sync + 'static {
    move |req: RouterEmbedRequest| {
        let iii = iii.clone();
        let registry = registry.clone();
        Box::pin(async move {
            if req.input.is_empty() {
                return Err(Error::Handler(
                    "router/invalid_input: input must not be empty".into(),
                ));
            }
            let candidates: Vec<String> = match req.provider.filter(|p| !p.trim().is_empty()) {
                Some(p) => vec![p],
                None => {
                    // Registered providers, `openai` preferred so the
                    // default choice is deterministic.
                    let mut ids = registry.ids().await;
                    ids.sort();
                    if let Some(pos) = ids.iter().position(|p| p == "openai") {
                        ids.swap(0, pos);
                    }
                    ids
                }
            };
            if candidates.is_empty() {
                return Err(Error::Handler(
                    "router/no_embed_provider: no providers are registered".into(),
                ));
            }
            for provider in &candidates {
                if let Some(resp) = try_provider(&iii, provider, &req.model, &req.input).await? {
                    return Ok(resp);
                }
            }
            Err(Error::Handler(format!(
                "router/no_embed_provider: none of the registered providers ({}) implement \
                 provider::<id>::embed",
                candidates.join(", ")
            )))
        })
    }
}

type BoxedEmbedFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterEmbedResponse, Error>> + Send>>;
