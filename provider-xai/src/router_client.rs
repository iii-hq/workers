//! Provider-scoped shims over the shared router-protocol client
//! (`llm_router::provider_scaffold::router_client`): every call binds this
//! crate's `PROVIDER_ID` and carries the registration token.
use crate::PROVIDER_ID;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::router_client as scaffold;
use llm_router::types::model::Model;
use llm_router::types::router::{DiscoveryReport, ProviderResolveResponse};
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

/// Begin a token-gated latest-wins discovery without changing streaming resolve.
pub async fn begin_discovery(
    iii: &IIIClient,
    token: Option<&str>,
) -> Result<(ProviderResolveResponse, Option<String>), Error> {
    let raw = scaffold::call(
        iii,
        "router::provider::resolve",
        serde_json::json!({
            "id": PROVIDER_ID, "token": token, "begin_discovery": true,
        }),
    )
    .await?;
    let attempt = raw
        .get("discovery_attempt")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let explicit_reference = raw.get("credential_source").and_then(Value::as_str) == Some("secret");
    let resolved = serde_json::from_value(raw)
        .map_err(|_| Error::Handler("invalid router discovery response".into()))?;
    let resolved = if explicit_reference {
        resolved
    } else {
        scaffold::apply_credential_env_fallback(resolved, Some(crate::register::CREDENTIAL_ENV_VAR))
    };
    Ok((resolved, attempt))
}

pub async fn complete_discovery(
    iii: &IIIClient,
    models: Vec<Model>,
    report: DiscoveryReport,
    token: Option<&str>,
) -> Result<usize, Error> {
    let raw = scaffold::call(
        iii,
        "router::models::reconcile",
        serde_json::json!({
            "provider": PROVIDER_ID, "models": models, "token": token, "discovery": report,
        }),
    )
    .await?;
    Ok(raw.get("count").and_then(Value::as_u64).unwrap_or(0) as usize)
}

/// `router::models::get` — authoritative catalog record (None when absent).
pub async fn models_get(iii: &IIIClient, model_id: &str) -> Option<Model> {
    scaffold::models_get(iii, PROVIDER_ID, model_id).await
}

/// `router::provider::register` — returns the registration token to persist.
pub async fn register(iii: &IIIClient, declaration: Value) -> Result<Value, Error> {
    scaffold::register(iii, declaration).await
}
