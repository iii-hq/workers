//! Thin wrappers over the router's provider-protocol functions. All calls
//! carry the registration token (identity binding, spec adaptation #1).
//! `provider_id` is the caller's declared provider id (e.g. "anthropic").
use crate::types::credential::Credential;
use crate::types::model::Model;
use crate::types::router::{CredentialSource, ProviderResolveResponse};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
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
    provider_id: &str,
    token: Option<&str>,
    credential_env_var: Option<&str>,
) -> Result<ProviderResolveResponse, Error> {
    let mut payload = json!({ "id": provider_id });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    let raw = call(iii, "router::provider::resolve", payload).await?;
    let explicit_reference = names_a_secret_reference(&raw);
    let resp: ProviderResolveResponse = serde_json::from_value(raw).map_err(|e| Error::Remote {
        code: "provider/bad_resolve_response".into(),
        message: e.to_string(),
        stacktrace: None,
    })?;
    if explicit_reference {
        // The operator chose a `secret://` or `env://` reference: when it does not
        // resolve, the router's `credential_error` stands rather than this
        // process's env key quietly taking over.
        return Ok(resp);
    }
    Ok(apply_credential_env_fallback(resp, credential_env_var))
}

/// Whether the router resolved (or failed to resolve) a `secret://` or
/// `env://` reference — `credential_source: "secret"`, absent from older
/// routers.
fn names_a_secret_reference(raw: &Value) -> bool {
    raw.get("credential_source").and_then(Value::as_str) == Some("secret")
}

/// `router::models::reconcile` — replace this provider's catalog slice.
pub async fn reconcile(
    iii: &IIIClient,
    provider_id: &str,
    models: Vec<Model>,
    token: Option<&str>,
) -> Result<(), Error> {
    let mut payload = json!({
        "provider": provider_id,
        "models": serde_json::to_value(models).expect("serializable models"),
    });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    call(iii, "router::models::reconcile", payload).await?;
    Ok(())
}

/// `router::models::get` — authoritative catalog record (None when absent).
pub async fn models_get(iii: &IIIClient, provider_id: &str, model_id: &str) -> Option<Model> {
    let raw = call(
        iii,
        "router::models::get",
        json!({ "provider": provider_id, "id": model_id }),
    )
    .await
    .ok()?;
    serde_json::from_value(raw.get("model")?.clone()).ok()
}

/// `router::provider::register` — returns the registration token to persist.
pub async fn register(iii: &IIIClient, declaration: Value) -> Result<Value, Error> {
    call(iii, "router::provider::register", declaration).await
}

/// Inject an env-sourced ApiKey only when the router resolved nothing.
/// Pure: takes the already-read value so tests never touch process env.
fn with_api_key_fallback(
    mut resp: ProviderResolveResponse,
    key: Option<String>,
) -> ProviderResolveResponse {
    if resp.credential.is_some() {
        return resp; // router / config credential always wins
    }
    if let Some(k) = key {
        let k = k.trim();
        if !k.is_empty() {
            resp.credential = Some(Credential::ApiKey { key: k.to_string() });
            resp.source = CredentialSource::Env;
            resp.configured = true;
        }
    }
    resp
}

/// Read the provider's declared env var and apply the fallback.
pub fn apply_credential_env_fallback(
    resp: ProviderResolveResponse,
    credential_env_var: Option<&str>,
) -> ProviderResolveResponse {
    let key = credential_env_var.and_then(|name| std::env::var(name).ok());
    with_api_key_fallback(resp, key)
}

#[cfg(test)]
mod fallback_tests {
    use super::{names_a_secret_reference, with_api_key_fallback};
    use crate::types::credential::Credential;
    use crate::types::router::{CredentialSource, ProviderResolveResponse};

    fn none_resp() -> ProviderResolveResponse {
        ProviderResolveResponse {
            configured: false,
            source: CredentialSource::None,
            credential: None,
            api_url: None,
            max_tokens: None,
        }
    }

    #[test]
    fn router_credential_wins_over_env() {
        let mut resp = none_resp();
        resp.credential = Some(Credential::ApiKey {
            key: "from-router".into(),
        });
        resp.source = CredentialSource::Config;
        resp.configured = true;
        let out = with_api_key_fallback(resp, Some("from-env".into()));
        assert_eq!(
            out.credential,
            Some(Credential::ApiKey {
                key: "from-router".into()
            })
        );
        assert_eq!(out.source, CredentialSource::Config);
    }

    #[test]
    fn injects_env_when_router_has_none() {
        let out = with_api_key_fallback(none_resp(), Some("sk-abc".into()));
        assert_eq!(
            out.credential,
            Some(Credential::ApiKey {
                key: "sk-abc".into()
            })
        );
        assert_eq!(out.source, CredentialSource::Env);
        assert!(out.configured);
    }

    #[test]
    fn no_key_leaves_none() {
        let out = with_api_key_fallback(none_resp(), None);
        assert_eq!(out.credential, None);
        assert!(!out.configured);
    }

    #[test]
    fn empty_and_whitespace_are_not_injected() {
        assert_eq!(
            with_api_key_fallback(none_resp(), Some("".into())).credential,
            None
        );
        assert_eq!(
            with_api_key_fallback(none_resp(), Some("  \n".into())).credential,
            None
        );
    }

    #[test]
    fn only_a_secret_reference_opts_out_of_the_env_fallback() {
        assert!(names_a_secret_reference(&serde_json::json!({
            "configured": false, "source": "none", "credential": null,
            "credential_source": "secret", "credential_ref": "secret://K",
            "credential_error": "secret K not found in the secrets worker",
        })));
        // older routers carry no credential_source
        assert!(!names_a_secret_reference(&serde_json::json!({
            "configured": false, "source": "none", "credential": null,
        })));
        assert!(!names_a_secret_reference(&serde_json::json!({
            "configured": false, "source": "none", "credential": null,
            "credential_source": "none",
        })));
    }

    #[test]
    fn injected_key_is_trimmed() {
        let out = with_api_key_fallback(none_resp(), Some(" sk-abc\n".into()));
        assert_eq!(
            out.credential,
            Some(Credential::ApiKey {
                key: "sk-abc".into()
            })
        );
    }
}
