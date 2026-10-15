//! `router::embed` — one front door for text embeddings, mirroring the
//! chat surface's shape: callers name a provider or let the router pick
//! one from its own provider registry. Providers implement
//! `provider::<id>::embed` (batch in, one vector per input, order
//! preserved); registered providers without the surface are skipped on a
//! function-not-found, so a new embed-capable provider needs zero router
//! changes to serve embeddings. An auto-pick also moves past a provider
//! that is not configured (no api key), but never past a transient failure.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::registry::store::RegistryStore;
use crate::types::errors::{is_function_not_found, is_provider_unconfigured};

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
            let explicit = req.provider.filter(|p| !p.trim().is_empty());
            let auto = explicit.is_none();
            let candidates: Vec<String> = match explicit {
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
            first_embedding(&candidates, auto, |provider| {
                let (iii, model, input) = (iii.clone(), req.model.clone(), req.input.clone());
                async move { try_provider(&iii, &provider, &model, &input).await }
            })
            .await
        })
    }
}

/// Walk the candidates in order. `Ok(None)` (no embed surface) moves on;
/// so does an unconfigured provider on an auto-pick (`auto`). Any other
/// error is returned as is: falling back past a transient failure would
/// embed the same caller's corpus into another model's vector space, and
/// callers cannot tell the spaces apart (they retry later instead).
async fn first_embedding<F, Fut>(
    candidates: &[String],
    auto: bool,
    mut attempt: F,
) -> Result<RouterEmbedResponse, Error>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<RouterEmbedResponse>, Error>>,
{
    let mut unconfigured = Vec::new();
    for provider in candidates {
        match attempt(provider.clone()).await {
            Ok(Some(resp)) => return Ok(resp),
            Ok(None) => {}
            Err(e) if auto && is_provider_unconfigured(&e) => unconfigured.push(provider.as_str()),
            Err(e) => return Err(e),
        }
    }
    let detail = if unconfigured.is_empty() {
        String::new()
    } else {
        format!("; not configured: {}", unconfigured.join(", "))
    };
    Err(Error::Handler(format!(
        "router/no_embed_provider: none of the registered providers ({}) can serve \
         provider::<id>::embed{detail}",
        candidates.join(", ")
    )))
}

type BoxedEmbedFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterEmbedResponse, Error>> + Send>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(provider: &str) -> RouterEmbedResponse {
        RouterEmbedResponse {
            provider: provider.into(),
            model: format!("{provider}-model"),
            embeddings: vec![vec![1.0]],
        }
    }

    fn remote(message: &str) -> Error {
        Error::Remote {
            code: "invocation_failed".into(),
            message: format!("handler error: {message}"),
            stacktrace: None,
        }
    }

    fn candidates() -> Vec<String> {
        vec!["openai".into(), "llamacpp".into()]
    }

    #[tokio::test]
    async fn an_auto_pick_moves_past_an_unconfigured_provider() {
        let resp = first_embedding(&candidates(), true, |p| async move {
            match p.as_str() {
                "openai" => Err(remote("provider/not_configured: no api_key")),
                _ => Ok(Some(ok(&p))),
            }
        })
        .await
        .unwrap();
        assert_eq!(resp.provider, "llamacpp");
    }

    #[tokio::test]
    async fn a_transient_failure_is_returned_not_fallen_back_from() {
        let err = first_embedding(&candidates(), true, |p| async move {
            match p.as_str() {
                "openai" => Err(remote("provider/upstream_status: 503")),
                _ => Ok(Some(ok(&p))),
            }
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("provider/upstream_status: 503"));
    }

    #[tokio::test]
    async fn an_explicit_provider_is_never_substituted() {
        let err = first_embedding(&["openai".to_string()], false, |_| async {
            Err(remote("provider/not_configured: no api_key"))
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("provider/not_configured"));
    }

    #[tokio::test]
    async fn none_left_names_the_unconfigured_ones() {
        let err = first_embedding(&candidates(), true, |p| async move {
            match p.as_str() {
                "openai" => Err(remote("provider/not_configured: no api_key")),
                _ => Ok(None),
            }
        })
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("router/no_embed_provider"), "{msg}");
        assert!(msg.contains("not configured: openai"), "{msg}");
    }
}
