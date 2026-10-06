//! The `router::provider::resolve` and `router::provider::update_credential`
//! iii functions. Credential precedence: stored slice (`credential` object →
//! literal `api_key` → `secret://NAME` or `env://NAME` reference, resolved
//! through the `secrets` worker) → declared env var → none. A reference that does not
//! resolve leaves the provider unconfigured with a `credential_error`: an
//! explicit reference wins, so the env var is NOT used as a fallback past it.
//!
//! Engine-backed coverage: tests/integration.rs (resolve precedence,
//! update_credential round-trip, secret reference lifecycle).
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::types::credential::Credential;
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{
    CredentialOrigin, CredentialSource, CredentialStatus, ProviderDeclaration,
    ProviderResolveOutput, ProviderResolveRequest, ProviderResolveResponse,
    UpdateCredentialRequest, UpdateCredentialResponse,
};
use futures::future::BoxFuture;
use iii_sdk::{errors::Error, IIIClient};
use serde_json::{json, Value};

use crate::config::entry::{read_entry_value, write_entry_value, EntryWriteLock};
use crate::config::state::{apply_config, snapshot, ConfigCell, ConfigSnapshot};
use crate::registry::store::RegistryStore;
use crate::secrets::{parse_ref, SecretCache, SecretError};

/// What a provider slice stores as its credential, first match wins.
#[derive(Debug, PartialEq)]
pub enum SliceCredential {
    /// The `credential` object (written by update_credential) or a literal
    /// `api_key`.
    Stored(Credential),
    /// A `secret://NAME` or `env://NAME` reference in `api_key` (or in an
    /// `api_key`-typed `credential`), canonical ([`SecretRef::key`]); `Err`
    /// when malformed. Never forwarded as a key.
    ///
    /// [`SecretRef::key`]: crate::secrets::SecretRef::key
    Reference(Result<String, SecretError>),
    Absent,
}

pub fn slice_credential(slice: &Value) -> SliceCredential {
    let reference =
        |key: &str| parse_ref(key).map(|r| SliceCredential::Reference(r.map(|r| r.key())));
    if let Ok(credential) = serde_json::from_value::<Credential>(
        slice.get("credential").cloned().unwrap_or(Value::Null),
    ) {
        if let Credential::ApiKey { key } = &credential {
            if let Some(reference) = reference(key) {
                return reference;
            }
        }
        return SliceCredential::Stored(credential);
    }
    match slice
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
    {
        Some(key) => reference(key).unwrap_or_else(|| {
            SliceCredential::Stored(Credential::ApiKey {
                key: key.to_string(),
            })
        }),
        None => SliceCredential::Absent,
    }
}

/// Canonical reference → ids of the providers whose slice references it.
pub fn referenced_secrets(config: &ConfigSnapshot) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let providers = config.value().get("providers").and_then(Value::as_object);
    for (id, slice) in providers.into_iter().flatten() {
        if let SliceCredential::Reference(Ok(reference)) = slice_credential(slice) {
            out.entry(reference).or_default().insert(id.clone());
        }
    }
    out
}

fn provider_reference(config: &ConfigSnapshot, provider: &str) -> Option<String> {
    match slice_credential(config.provider_slice(provider)?) {
        SliceCredential::Reference(Ok(reference)) => Some(reference),
        _ => None,
    }
}

/// Resolve `provider`'s reference on a cache miss, so the sync precedence
/// below can read it. Async paths call this first.
pub async fn ensure_provider_secret(
    config: &ConfigSnapshot,
    provider: &str,
    secrets: &SecretCache,
) {
    if let Some(reference) = provider_reference(config, provider) {
        secrets.ensure([reference]).await;
    }
}

/// `Some(reason)` when `provider`'s slice holds a reference that does not
/// resolve — what `router::chat` reports instead of dispatching to
/// a provider that would only say "not configured".
pub async fn unresolved_reference(
    config: &ConfigSnapshot,
    provider: &str,
    secrets: &SecretCache,
) -> Option<String> {
    ensure_provider_secret(config, provider, secrets).await;
    let slice = config.provider_slice(provider)?;
    match slice_credential(slice) {
        SliceCredential::Reference(Ok(reference)) => match secrets.lookup(&reference)? {
            Ok(_) => None,
            Err(error) => Some(error.describe(&reference)),
        },
        SliceCredential::Reference(Err(error)) => Some(error.describe("")),
        _ => None,
    }
}

/// Core resolution — shared by the resolve handler, provider::list, and chat.
/// Sync over the snapshot and the secret cache; references the cache has not
/// resolved yet report so (async callers `ensure` them first).
pub fn resolve_provider_config(
    config: &ConfigSnapshot,
    declaration: &ProviderDeclaration,
    secrets: &SecretCache,
) -> ProviderResolveOutput {
    let slice = config
        .provider_slice(&declaration.id)
        .cloned()
        .unwrap_or(Value::Null);

    let api_url = slice
        .get("api_url")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            declaration
                .defaults
                .as_ref()
                .and_then(|d| d.api_url.clone())
        });
    let max_tokens = slice
        .get("max_tokens")
        .and_then(Value::as_u64)
        .or_else(|| declaration.defaults.as_ref().and_then(|d| d.max_tokens));

    let status = |origin: CredentialOrigin| CredentialStatus {
        credential_source: Some(origin),
        ..CredentialStatus::default()
    };
    let (credential, source, status) = match slice_credential(&slice) {
        SliceCredential::Stored(credential) => (
            Some(credential),
            CredentialSource::Config,
            status(CredentialOrigin::Config),
        ),
        SliceCredential::Reference(reference) => {
            let credential_ref = reference.as_ref().ok().cloned();
            let resolved = match &reference {
                Ok(reference) => match secrets.lookup(reference) {
                    Some(Ok(key)) => Ok(key),
                    Some(Err(error)) => Err(error.describe(reference)),
                    None => Err(format!("{reference} has not been resolved yet")),
                },
                Err(error) => Err(error.describe("")),
            };
            match resolved {
                // `source` stays `config` for providers: the reference lives
                // in the configuration entry. `credential_source` is
                // `secret` for both schemes — the secrets worker resolved it.
                Ok(key) => (
                    Some(Credential::ApiKey { key }),
                    CredentialSource::Config,
                    CredentialStatus {
                        credential_ref,
                        ..status(CredentialOrigin::Secret)
                    },
                ),
                Err(error) => (
                    None,
                    CredentialSource::None,
                    CredentialStatus {
                        credential_ref,
                        credential_error: Some(error),
                        ..status(CredentialOrigin::Secret)
                    },
                ),
            }
        }
        SliceCredential::Absent => match declaration
            .credential_env_var
            .as_ref()
            .and_then(|var| std::env::var(var).ok())
            .filter(|k| !k.is_empty())
        {
            Some(env_key) => (
                Some(Credential::ApiKey { key: env_key }),
                CredentialSource::Env,
                status(CredentialOrigin::Env),
            ),
            None => (None, CredentialSource::None, status(CredentialOrigin::None)),
        },
    };

    ProviderResolveOutput {
        resolved: ProviderResolveResponse {
            configured: credential.is_some(),
            source,
            credential,
            api_url,
            max_tokens,
        },
        status,
    }
}

pub fn make_provider_resolve(
    config: ConfigCell,
    registry: Arc<RegistryStore>,
    secrets: Arc<SecretCache>,
) -> impl Fn(ProviderResolveRequest) -> BoxFuture<'static, Result<ProviderResolveOutput, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ProviderResolveRequest| {
        let (config, registry, secrets) = (config.clone(), registry.clone(), secrets.clone());
        Box::pin(async move {
            let record = registry
                .verify_token(&req.id, req.token.as_deref())
                .await
                .map_err(Error::from)?;
            let config = snapshot(&config);
            ensure_provider_secret(&config, &record.declaration.id, &secrets).await;
            Ok(resolve_provider_config(
                &config,
                &record.declaration,
                &secrets,
            ))
        })
    }
}

/// OAuth write-back (spec § update_credential): providers never write the
/// configuration entry directly. Read-merge-write under the entry lock.
pub fn make_update_credential(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    config: ConfigCell,
    entry_lock: EntryWriteLock,
) -> impl Fn(UpdateCredentialRequest) -> BoxFuture<'static, Result<UpdateCredentialResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: UpdateCredentialRequest| {
        let (iii, registry, config, entry_lock) = (
            iii.clone(),
            registry.clone(),
            config.clone(),
            entry_lock.clone(),
        );
        Box::pin(async move {
            registry
                .verify_token(&req.id, req.token.as_deref())
                .await
                .map_err(Error::from)?;
            let credential = req.credential;
            if !credential.is_object() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "credential object is required",
                )
                .into());
            }
            let _guard = entry_lock.lock().await;
            let mut entry = read_entry_value(&iii).await?;
            if !entry.is_object() {
                entry = json!({});
            }
            let providers = entry
                .as_object_mut()
                .expect("object")
                .entry("providers")
                .or_insert_with(|| json!({}));
            let slice = providers
                .as_object_mut()
                .expect("object")
                .entry(&req.id)
                .or_insert_with(|| json!({}));
            slice["credential"] = credential;
            write_entry_value(&iii, entry.clone()).await?;
            // Make this worker-originated write visible immediately; the
            // asynchronous configuration trigger will subsequently re-fetch
            // the same authoritative value and drive model discovery.
            apply_config(&config, entry);
            Ok(UpdateCredentialResponse { ok: true })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::testing::FakeSecrets;

    fn declaration(env_var: Option<&str>) -> ProviderDeclaration {
        ProviderDeclaration {
            id: "anthropic".into(),
            display_name: None,
            credential_env_var: env_var.map(String::from),
            defaults: None,
            config_schema: None,
            supports_model_listing: None,
            models: None,
            worker_id: None,
            icon_svg: None,
        }
    }

    fn config(slice: Value) -> ConfigSnapshot {
        ConfigSnapshot::from_value(json!({ "providers": { "anthropic": slice } }))
    }

    async fn resolve(
        slice: Value,
        env_var: Option<&str>,
        fake: &FakeSecrets,
    ) -> ProviderResolveOutput {
        let config = config(slice);
        let secrets = fake.cache();
        ensure_provider_secret(&config, "anthropic", &secrets).await;
        resolve_provider_config(&config, &declaration(env_var), &secrets)
    }

    fn api_key(output: &ProviderResolveOutput) -> Option<&str> {
        match &output.resolved.credential {
            Some(Credential::ApiKey { key }) => Some(key),
            _ => None,
        }
    }

    #[tokio::test]
    async fn stored_credential_then_literal_key_then_env_then_none() {
        let fake = FakeSecrets::default();
        let env = "LLM_ROUTER_UNIT_PRECEDENCE_KEY";
        std::env::set_var(env, "sk-env");

        let out = resolve(
            json!({ "credential": { "type": "oauth", "access_token": "at" }, "api_key": "sk-literal" }),
            Some(env),
            &fake,
        )
        .await;
        assert!(matches!(
            out.resolved.credential,
            Some(Credential::Oauth { .. })
        ));
        assert_eq!(out.status.credential_source, Some(CredentialOrigin::Config));

        let out = resolve(json!({ "api_key": "sk-literal" }), Some(env), &fake).await;
        assert_eq!(api_key(&out), Some("sk-literal"));
        assert_eq!(out.resolved.source, CredentialSource::Config);
        assert_eq!(out.status.credential_source, Some(CredentialOrigin::Config));
        assert_eq!(out.status.credential_ref, None);

        let out = resolve(json!({ "api_url": "https://x" }), Some(env), &fake).await;
        assert_eq!(api_key(&out), Some("sk-env"));
        assert_eq!(out.resolved.source, CredentialSource::Env);
        assert_eq!(out.status.credential_source, Some(CredentialOrigin::Env));

        std::env::remove_var(env);
        let out = resolve(json!({}), Some(env), &fake).await;
        assert!(!out.resolved.configured);
        assert_eq!(out.resolved.source, CredentialSource::None);
        assert_eq!(out.status.credential_source, Some(CredentialOrigin::None));
        assert!(fake.calls().is_empty(), "no reference, no secrets call");
    }

    #[tokio::test]
    async fn a_resolved_reference_supplies_the_key() {
        let fake = FakeSecrets::default();
        fake.set("secret://ANTHROPIC_API_KEY", Ok("sk-from-secrets"));
        let out = resolve(
            json!({ "api_key": "secret://ANTHROPIC_API_KEY" }),
            None,
            &fake,
        )
        .await;
        assert!(out.resolved.configured);
        assert_eq!(api_key(&out), Some("sk-from-secrets"));
        // providers keep seeing `config`; the diagnostics say `secret`
        assert_eq!(out.resolved.source, CredentialSource::Config);
        assert_eq!(out.status.credential_source, Some(CredentialOrigin::Secret));
        assert_eq!(
            out.status.credential_ref.as_deref(),
            Some("secret://ANTHROPIC_API_KEY")
        );
        assert_eq!(out.status.credential_error, None);
    }

    #[tokio::test]
    async fn an_unresolvable_reference_is_not_configured_and_never_falls_back_to_env() {
        let env = "LLM_ROUTER_UNIT_REF_FALLBACK_KEY";
        std::env::set_var(env, "sk-env-must-not-win");
        let cases = [
            (
                Err(SecretError::NotFound),
                "secret ANTHROPIC_API_KEY not found in the secrets worker",
            ),
            (
                Err(SecretError::Forbidden),
                "llm-router is not allowed to read secret ANTHROPIC_API_KEY; add llm-router to the secret's consumers",
            ),
            (
                Err(SecretError::Unavailable),
                "secrets worker is not running",
            ),
        ];
        for (outcome, expected) in cases {
            let fake = FakeSecrets::default();
            fake.set("secret://ANTHROPIC_API_KEY", outcome);
            let out = resolve(
                json!({ "api_key": "secret://ANTHROPIC_API_KEY" }),
                Some(env),
                &fake,
            )
            .await;
            assert!(!out.resolved.configured, "{expected}");
            assert_eq!(out.resolved.credential, None);
            assert_eq!(out.resolved.source, CredentialSource::None);
            assert_eq!(out.status.credential_source, Some(CredentialOrigin::Secret));
            assert_eq!(
                out.status.credential_ref.as_deref(),
                Some("secret://ANTHROPIC_API_KEY")
            );
            let error = out.status.credential_error.expect("actionable error");
            assert!(error.starts_with(expected), "{error}");
        }
        std::env::remove_var(env);
    }

    #[tokio::test]
    async fn an_env_reference_resolves_through_the_secrets_worker() {
        // The router's own variable of the same name never stands in for it.
        let env = "LLM_ROUTER_UNIT_ENV_REF_KEY";
        std::env::set_var(env, "sk-router-process-env");
        let fake = FakeSecrets::default();
        fake.set("env://LLM_ROUTER_UNIT_ENV_REF_KEY", Ok("sk-from-dotenv"));
        let out = resolve(
            json!({ "api_key": "env://LLM_ROUTER_UNIT_ENV_REF_KEY" }),
            Some(env),
            &fake,
        )
        .await;
        assert!(out.resolved.configured);
        assert_eq!(api_key(&out), Some("sk-from-dotenv"));
        assert_eq!(out.status.credential_source, Some(CredentialOrigin::Secret));
        assert_eq!(
            out.status.credential_ref.as_deref(),
            Some("env://LLM_ROUTER_UNIT_ENV_REF_KEY")
        );
        assert_eq!(fake.calls(), vec!["env://LLM_ROUTER_UNIT_ENV_REF_KEY"]);

        let fake = FakeSecrets::default();
        fake.set(
            "env://LLM_ROUTER_UNIT_ENV_REF_KEY",
            Err(SecretError::NotFound),
        );
        let out = resolve(
            json!({ "api_key": "env://LLM_ROUTER_UNIT_ENV_REF_KEY" }),
            Some(env),
            &fake,
        )
        .await;
        assert!(!out.resolved.configured);
        assert_eq!(out.resolved.credential, None);
        assert!(out
            .status
            .credential_error
            .unwrap()
            .starts_with("environment variable LLM_ROUTER_UNIT_ENV_REF_KEY is not set"));
        std::env::remove_var(env);
    }

    #[tokio::test]
    async fn a_reference_is_never_forwarded_as_a_literal_key() {
        let fake = FakeSecrets::default(); // every lookup: secrets worker absent
        for slice in [
            json!({ "api_key": "secret://ANTHROPIC_API_KEY" }),
            json!({ "api_key": " SECRET://ANTHROPIC_API_KEY\n" }),
            json!({ "api_key": "secret://not a name" }),
            json!({ "api_key": "secret://" }),
            json!({ "credential": { "type": "api_key", "key": "secret://ANTHROPIC_API_KEY" } }),
            json!({ "api_key": "env://ANTHROPIC_API_KEY" }),
            json!({ "api_key": "ENV://not a name" }),
        ] {
            let out = resolve(slice.clone(), None, &fake).await;
            assert_eq!(out.resolved.credential, None, "{slice}");
            assert!(!out.resolved.configured, "{slice}");
            assert!(out.status.credential_error.is_some(), "{slice}");
            let wire = serde_json::to_string(&out.resolved).unwrap().to_lowercase();
            assert!(
                !wire.contains("secret://") && !wire.contains("env://"),
                "{wire}"
            );
        }
    }

    #[tokio::test]
    async fn a_malformed_reference_is_reported_without_echoing_it() {
        let fake = FakeSecrets::default();
        let out = resolve(
            json!({ "api_key": "secret://sk ant pasted key" }),
            None,
            &fake,
        )
        .await;
        assert_eq!(out.status.credential_ref, None);
        let error = out.status.credential_error.unwrap();
        assert!(error.contains("malformed reference"), "{error}");
        assert!(!error.contains("pasted"), "{error}");
        assert!(fake.calls().is_empty(), "malformed references are not sent");
    }

    #[test]
    fn an_unresolved_reference_reports_it_until_ensured() {
        let fake = FakeSecrets::default();
        let out = resolve_provider_config(
            &config(json!({ "api_key": "secret://LATER" })),
            &declaration(None),
            &fake.cache(),
        );
        assert!(!out.resolved.configured);
        assert_eq!(
            out.status.credential_error.as_deref(),
            Some("secret://LATER has not been resolved yet")
        );
    }

    #[test]
    fn referenced_secrets_maps_names_to_providers() {
        let config = ConfigSnapshot::from_value(json!({ "providers": {
            "a": { "api_key": "secret://SHARED" },
            "b": { "api_key": "secret://SHARED" },
            "c": { "credential": { "type": "api_key", "key": "secret://C_KEY" } },
            "d": { "api_key": "sk-literal" },
            "e": { "api_key": "secret://bad name" },
            "f": "not a slice",
            "g": { "api_key": "env://E_KEY" },
        }}));
        let refs = referenced_secrets(&config);
        assert_eq!(
            refs.keys().cloned().collect::<Vec<_>>(),
            vec!["env://E_KEY", "secret://C_KEY", "secret://SHARED"]
        );
        assert_eq!(
            refs["secret://SHARED"].iter().cloned().collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[tokio::test]
    async fn chat_preflight_names_the_unresolvable_reference() {
        let fake = FakeSecrets::default();
        fake.set("secret://OK_KEY", Ok("v"));
        fake.set("secret://GONE", Err(SecretError::NotFound));
        let secrets = fake.cache();
        let config = ConfigSnapshot::from_value(json!({ "providers": {
            "ok": { "api_key": "secret://OK_KEY" },
            "gone": { "api_key": "secret://GONE" },
            "literal": { "api_key": "sk" },
        }}));
        assert_eq!(unresolved_reference(&config, "ok", &secrets).await, None);
        assert_eq!(
            unresolved_reference(&config, "literal", &secrets).await,
            None
        );
        assert_eq!(
            unresolved_reference(&config, "absent", &secrets).await,
            None
        );
        assert!(unresolved_reference(&config, "gone", &secrets)
            .await
            .unwrap()
            .starts_with("secret GONE not found"));
    }
}
