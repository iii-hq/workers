//! Catalog reconcile. Z.AI exposes no models-listing endpoint, so the curated
//! table (curated.rs) is the source of truth for the id list; refresh pushes
//! it through the router's single write path. The configured credential gates
//! the slice: no key → empty catalog, so the picker never shows unusable rows.
use crate::{curated, router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};

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

    let models = curated::models();
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
