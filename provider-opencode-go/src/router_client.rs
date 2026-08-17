//! Provider-scoped shims over the shared router-protocol client
//! (`llm_router::provider_scaffold::router_client`): the resolve, reconcile,
//! and models_get wrappers bind this crate's `PROVIDER_ID` and carry the
//! registration token. `register` forwards a declaration payload that already
//! carries both (see `register::declare_once`).
use crate::PROVIDER_ID;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::router_client as scaffold;
use llm_router::types::model::Model;
use llm_router::types::router::ProviderResolveResponse;
use serde_json::Value;

/// `router::provider::resolve` — credential + effective settings.
pub async fn resolve(
    iii: &IIIClient,
    token: Option<&str>,
) -> Result<ProviderResolveResponse, Error> {
    scaffold::resolve(
        iii,
        PROVIDER_ID,
        token,
        Some(crate::register::CREDENTIAL_ENV_VAR),
    )
    .await
}

/// `router::models::reconcile` — replace this provider's catalog slice.
pub async fn reconcile(
    iii: &IIIClient,
    models: Vec<Model>,
    token: Option<&str>,
) -> Result<(), Error> {
    scaffold::reconcile(iii, PROVIDER_ID, models, token).await
}

/// `router::models::get` — authoritative catalog record (None when absent).
pub async fn models_get(iii: &IIIClient, model_id: &str) -> Option<Model> {
    scaffold::models_get(iii, PROVIDER_ID, model_id).await
}

/// `router::provider::register` — returns the registration token to persist.
pub async fn register(iii: &IIIClient, declaration: Value) -> Result<Value, Error> {
    scaffold::register(iii, declaration).await
}
