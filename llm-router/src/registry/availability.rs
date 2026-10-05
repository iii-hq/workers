//! `router::provider::list` — registered providers with configured/available
//! status and credential diagnostics (`credential_source`, `credential_ref`,
//! `credential_error`).
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListRequest, ProviderListResponse};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::config::state::{snapshot, ConfigCell};
use crate::registry::resolve::{referenced_secrets, resolve_provider_config};
use crate::registry::store::RegistryStore;
use crate::secrets::SecretCache;

pub fn make_provider_list(
    config: ConfigCell,
    registry: Arc<RegistryStore>,
    secrets: Arc<SecretCache>,
) -> impl Fn(ProviderListRequest) -> BoxFuture<'static, Result<ProviderListResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: ProviderListRequest| {
        let (config, registry, secrets) = (config.clone(), registry.clone(), secrets.clone());
        Box::pin(async move {
            let config = snapshot(&config);
            // One concurrent round for every uncached reference.
            secrets
                .ensure(referenced_secrets(&config).into_keys())
                .await;
            let mut providers = Vec::new();
            for rec in registry.list().await {
                let resolved = resolve_provider_config(&config, &rec.declaration, &secrets);
                providers.push(ProviderInfo {
                    id: rec.declaration.id.clone(),
                    display_name: rec
                        .declaration
                        .display_name
                        .clone()
                        .unwrap_or_else(|| rec.declaration.id.clone()),
                    credential_env_var: rec.declaration.credential_env_var.clone(),
                    configured: resolved.resolved.configured,
                    available: rec.available,
                    supports_model_listing: rec.declaration.supports_model_listing.unwrap_or(false),
                    icon_svg: rec.declaration.icon_svg.clone(),
                    credential: resolved.status,
                });
            }
            providers.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(ProviderListResponse { providers })
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::types::router::{CredentialOrigin, CredentialStatus, ProviderInfo};
    use serde_json::json;

    fn info(credential: CredentialStatus) -> ProviderInfo {
        ProviderInfo {
            id: "anthropic".into(),
            display_name: "Anthropic".into(),
            credential_env_var: Some("ANTHROPIC_API_KEY".into()),
            configured: false,
            available: true,
            supports_model_listing: true,
            icon_svg: None,
            credential,
        }
    }

    #[test]
    fn list_entries_carry_flat_optional_credential_fields() {
        let wire = serde_json::to_value(info(CredentialStatus {
            credential_source: Some(CredentialOrigin::Secret),
            credential_ref: Some("secret://ANTHROPIC_API_KEY".into()),
            credential_error: Some("secrets worker is not running".into()),
        }))
        .unwrap();
        assert_eq!(wire["credential_source"], "secret");
        assert_eq!(wire["credential_ref"], "secret://ANTHROPIC_API_KEY");
        assert_eq!(wire["credential_error"], "secrets worker is not running");

        // Absent diagnostics stay off the wire, and an older router's entry
        // (no such keys) still parses.
        let bare = serde_json::to_value(info(CredentialStatus::default())).unwrap();
        for key in ["credential_source", "credential_ref", "credential_error"] {
            assert!(bare.get(key).is_none(), "{key}");
        }
        let old: ProviderInfo = serde_json::from_value(json!({
            "id": "a", "display_name": "A", "credential_env_var": null,
            "configured": true, "available": true, "supports_model_listing": false,
        }))
        .unwrap();
        assert_eq!(old.credential, CredentialStatus::default());
    }

    #[test]
    fn every_credential_source_serializes_lowercase() {
        for (origin, wire) in [
            (CredentialOrigin::Config, "config"),
            (CredentialOrigin::Env, "env"),
            (CredentialOrigin::Secret, "secret"),
            (CredentialOrigin::None, "none"),
        ] {
            assert_eq!(serde_json::to_value(origin).unwrap(), json!(wire));
        }
    }
}
