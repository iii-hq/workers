//! Catalog reconcile. Z.AI exposes no models-listing endpoint, so the curated
//! table (curated.rs) is the source of truth for the id list; refresh pushes
//! it through the router's single write path. The configured credential gates
//! the slice (no key → empty catalog, so the picker never shows unusable
//! rows), and the resolved endpoint picks the slice: the Coding Plan
//! endpoint serves only the plan's models.
use crate::config::DEFAULT_API_URL;
use crate::{curated, router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};

/// The catalog slice for a resolved endpoint: the GLM Coding Plan endpoint
/// serves only the plan's models; anywhere else (the general pay-as-you-go
/// endpoint, custom OpenAI-compatible servers) gets the full table.
pub fn catalog_for(api_url: &str) -> Vec<Model> {
    if api_url.contains("/api/coding/") {
        curated::coding_models()
    } else {
        curated::models()
    }
}

/// The refresh flow; returns the reconciled slice size.
pub async fn refresh_models(iii: &IIIClient) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    if resolved.credential.is_none() {
        // Key removed: prune the slice so the picker reflects removal
        // instead of showing stale, unusable rows.
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    }

    let api_url = resolved
        .api_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_API_URL);
    let models = catalog_for(api_url);
    let count = models.len();
    router_client::reconcile(iii, models, token.as_deref()).await?;
    Ok(count)
}

pub fn make_refresh_models(
    iii: IIIClient,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let iii = iii.clone();
        Box::pin(async move {
            let count = refresh_models(&iii).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_picks_the_catalog_slice() {
        // default (coding) endpoint → only the plan's models
        let coding: Vec<String> = catalog_for(DEFAULT_API_URL)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            coding,
            [
                "glm-5.2",
                "glm-5.1",
                "glm-5",
                "glm-5-turbo",
                "glm-4.7",
                "glm-4.5-air"
            ]
        );
        // general pay-as-you-go endpoint and custom servers → full table
        assert_eq!(
            catalog_for("https://api.z.ai/api/paas/v4/chat/completions").len(),
            curated::models().len()
        );
        assert_eq!(
            catalog_for("http://127.0.0.1:8000/v1/chat/completions").len(),
            curated::models().len()
        );
    }
}
