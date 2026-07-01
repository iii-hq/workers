//! Catalog reconciliation. The ChatGPT/Codex backend has no usable
//! `GET /v1/models`, so the slice is the static, namespaced `curated::static_models`
//! set — reconciled through the router's single write path. No HTTP.
use crate::{curated, router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};

/// Reconcile the static catalog slice; returns its size.
pub async fn refresh_models(iii: &IIIClient, _http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let models = curated::static_models();
    let count = models.len();
    router_client::reconcile(iii, models, token.as_deref()).await?;
    Ok(count)
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}
