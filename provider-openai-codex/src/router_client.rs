//! Provider-scoped router shims and optional compatibility with an external
//! credential vault. Native authentication is owned by `session::AuthManager`.
use crate::PROVIDER_ID;
use iii_sdk::engine::EngineFunctions;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::router_client::{self as scaffold, call};
use llm_router::types::model::Model;
use llm_router::types::router::ProviderResolveResponse;
use serde_json::{json, Value};

const AUTH_GET_TOKEN_FN: &str = "auth::get_token";

fn function_prefix(function_id: &str) -> &str {
    function_id
        .split_once("::")
        .map(|(prefix, _)| prefix)
        .unwrap_or(function_id)
}

fn list_contains_function(raw: &Value, function_id: &str, namespace: &str) -> bool {
    let items: Vec<&Value> = match raw {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .get("functions")
            .or_else(|| map.get("items"))
            .and_then(Value::as_array)
            .map(|items| items.iter().collect())
            .unwrap_or_else(|| map.values().collect()),
        _ => return false,
    };
    items.iter().any(|item| {
        let id_matches = item
            .get("function_id")
            .or_else(|| item.get("id"))
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            == Some(function_id);
        let namespace_matches = item
            .get("namespace")
            .and_then(Value::as_str)
            .is_none_or(|value| value == namespace);
        id_matches && namespace_matches
    })
}

async fn function_available(iii: &IIIClient, function_id: &str) -> Result<bool, Error> {
    let prefix = format!("{}::", function_prefix(function_id));
    let raw = call(
        iii,
        EngineFunctions::LIST_FUNCTIONS,
        json!({ "prefix": prefix }),
    )
    .await?;
    let namespace = iii.namespace().unwrap_or_else(|| "default".into());
    Ok(list_contains_function(&raw, function_id, &namespace))
}

pub async fn auth_get_token_available(iii: &IIIClient) -> Result<bool, Error> {
    function_available(iii, AUTH_GET_TOKEN_FN).await
}

/// `router::provider::resolve` — effective settings (api_url / max_tokens).
/// The `credential` field is unused here; the vault is the credential source.
pub async fn resolve(
    iii: &IIIClient,
    token: Option<&str>,
) -> Result<ProviderResolveResponse, Error> {
    scaffold::resolve(iii, PROVIDER_ID, token, None).await
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

// ── auth-credentials vault ──────────────────────────────────────────────────

/// `auth::get_token` — the vault's runtime credential (None when absent). The
/// vault refreshes an expiring OAuth token on resolve, so this returns a fresh
/// access token; the returned JSON shape is parsed leniently in `config`.
pub async fn get_token(iii: &IIIClient, provider: &str) -> Result<Option<Value>, Error> {
    let raw = call(iii, "auth::get_token", json!({ "provider": provider })).await?;
    Ok(if raw.is_null() { None } else { Some(raw) })
}

/// Optional vault lookup. When the auth vault is not registered, skip the call
/// entirely so the engine does not log an expected `function_not_found`.
pub async fn get_token_if_available(
    iii: &IIIClient,
    provider: &str,
) -> Result<Option<Value>, Error> {
    if !auth_get_token_available(iii).await? {
        return Ok(None);
    }
    get_token(iii, provider).await
}

/// `oauth::openai-codex::refresh` — vault/oauth-worker-owned refresh. The
/// provider only *triggers* it (never calls the OAuth endpoints itself).
pub async fn refresh(iii: &IIIClient, provider: &str) -> Result<(), Error> {
    call(
        iii,
        crate::auth::REFRESH_FN_ID,
        json!({ "provider": provider }),
    )
    .await?;
    Ok(())
}

/// Optional OAuth refresh. Returns `Ok(false)` when the refresh worker is not
/// registered.
pub async fn refresh_if_available(iii: &IIIClient, provider: &str) -> Result<bool, Error> {
    if !function_available(iii, crate::auth::REFRESH_FN_ID).await? {
        return Ok(false);
    }
    refresh(iii, provider).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::list_contains_function;
    use serde_json::json;

    #[test]
    fn function_list_parser_accepts_engine_shapes() {
        assert!(list_contains_function(
            &json!({ "functions": [{ "function_id": "auth::get_token" }] }),
            "auth::get_token",
            "default"
        ));
        assert!(list_contains_function(
            &json!([{ "id": "auth::set_token" }]),
            "auth::set_token",
            "default"
        ));
        assert!(list_contains_function(
            &json!({ "items": [{ "name": "oauth::openai-codex::refresh" }] }),
            "oauth::openai-codex::refresh",
            "default"
        ));
        assert!(!list_contains_function(
            &json!({ "functions": [{
                "function_id": "auth::get_token",
                "namespace": "default"
            }] }),
            "auth::get_token",
            "project-a"
        ));
    }

    #[test]
    fn function_list_parser_rejects_missing_or_malformed_entries() {
        assert!(!list_contains_function(
            &json!({ "functions": [{ "function_id": "auth::status" }] }),
            "auth::get_token",
            "default"
        ));
        assert!(!list_contains_function(
            &json!({ "functions": [{ "id": 1 }] }),
            "auth::get_token",
            "default"
        ));
        assert!(!list_contains_function(
            &json!(null),
            "auth::get_token",
            "default"
        ));
    }
}
