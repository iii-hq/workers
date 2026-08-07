//! Thin wrappers over the router's provider-protocol functions. All calls
//! carry the registration token (identity binding, spec adaptation #1).
use crate::PROVIDER_ID;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::ProviderResolveResponse;
use serde_json::{json, Value};

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(15_000),
    })
    .await
}

/// `router::provider::resolve` — credential + effective settings.
pub async fn resolve(
    iii: &IIIClient,
    token: Option<&str>,
) -> Result<ProviderResolveResponse, Error> {
    let mut payload = json!({ "id": PROVIDER_ID });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    let raw = call(iii, "router::provider::resolve", payload).await?;
    let resp: ProviderResolveResponse = serde_json::from_value(raw).map_err(|e| Error::Remote {
        code: "provider/bad_resolve_response".into(),
        message: e.to_string(),
        stacktrace: None,
    })?;
    Ok(
        llm_router::provider_scaffold::router_client::apply_credential_env_fallback(
            resp,
            Some(crate::register::CREDENTIAL_ENV_VAR),
        ),
    )
}

/// Serializes whole-slice replacement. `reconcile` overwrites the provider's
/// entire catalog, so a prune's read-modify-write racing another prune (or a
/// refresh) would drop one of the two results.
static CATALOG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `router::models::reconcile` under the catalog lock — use this for every
/// caller that computes a slice and then writes it.
pub async fn reconcile_exclusive(
    iii: &IIIClient,
    models: Vec<Model>,
    token: Option<&str>,
) -> Result<(), Error> {
    let _guard = CATALOG_LOCK.lock().await;
    reconcile(iii, models, token).await
}

/// `router::models::reconcile` — replace this provider's catalog slice.
pub async fn reconcile(
    iii: &IIIClient,
    models: Vec<Model>,
    token: Option<&str>,
) -> Result<(), Error> {
    let mut payload = json!({
        "provider": PROVIDER_ID,
        "models": serde_json::to_value(models).expect("serializable models"),
    });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    call(iii, "router::models::reconcile", payload).await?;
    Ok(())
}

/// Drop one model from this provider's catalog slice.
///
/// The Copilot listing advertises models the account cannot actually call:
/// entitlement is per-plan (premium models on a plan without premium
/// requests answer `model_not_supported`) and no listing field exposes it.
/// Rather than guess, the slice self-heals — the first upstream refusal
/// removes the row, so the picker converges on what this account can really
/// use. A later refresh restores it if the entitlement changes.
pub async fn prune_model(iii: &IIIClient, model_id: &str, token: Option<&str>) {
    // Held across the read and the write: two models failing at once must not
    // each write back a snapshot that still contains the other.
    let _guard = CATALOG_LOCK.lock().await;
    let Ok(raw) = call(
        iii,
        "router::models::list",
        json!({ "provider": PROVIDER_ID }),
    )
    .await
    else {
        return;
    };
    let Some(models) = raw
        .get("models")
        .and_then(|m| serde_json::from_value::<Vec<Model>>(m.clone()).ok())
    else {
        return;
    };
    let kept: Vec<Model> = models.into_iter().filter(|m| m.id != model_id).collect();
    if let Err(e) = reconcile(iii, kept, token).await {
        eprintln!("[provider-github-copilot] pruning {model_id} failed ({e})");
    } else {
        println!(
            "[provider-github-copilot] {model_id} is not available on this plan — removed from the catalog"
        );
    }
}

/// `router::models::get` — authoritative catalog record (None when absent).
pub async fn models_get(iii: &IIIClient, model_id: &str) -> Option<Model> {
    let raw = call(
        iii,
        "router::models::get",
        json!({ "provider": PROVIDER_ID, "id": model_id }),
    )
    .await
    .ok()?;
    serde_json::from_value(raw.get("model")?.clone()).ok()
}

/// `router::provider::register` — returns the registration token to persist.
pub async fn register(iii: &IIIClient, declaration: Value) -> Result<Value, Error> {
    call(iii, "router::provider::register", declaration).await
}
