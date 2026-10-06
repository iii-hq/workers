//! `router::provider::list` — registered providers with configured/available
//! status and credential diagnostics (`credential_source`, `credential_ref`,
//! `credential_error`).
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListRequest, ProviderListResponse};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::catalog::store::CatalogStore;
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::resolve::{referenced_secrets, resolve_provider_config};
use crate::registry::store::RegistryStore;
use crate::secrets::SecretCache;

pub fn make_provider_list(
    config: ConfigCell,
    registry: Arc<RegistryStore>,
    secrets: Arc<SecretCache>,
    catalog: Arc<CatalogStore>,
) -> impl Fn(ProviderListRequest) -> BoxFuture<'static, Result<ProviderListResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: ProviderListRequest| {
        let (config, registry, secrets, catalog) = (
            config.clone(),
            registry.clone(),
            secrets.clone(),
            catalog.clone(),
        );
        Box::pin(async move {
            let config = snapshot(&config);
            // One concurrent round for every uncached reference.
            secrets
                .ensure(referenced_secrets(&config).into_keys())
                .await;
            let mut providers = Vec::new();
            for rec in registry.list().await {
                let resolved = resolve_provider_config(&config, &rec.declaration, &secrets);
                // Resolved at read time: slices fill from the first live
                // model refresh, after registration.
                let chat_ids: Vec<String> = catalog
                    .slice(&rec.declaration.id)
                    .await
                    .into_iter()
                    .filter(|model| model.speech.is_none())
                    .map(|model| model.id)
                    .collect();
                let default_model = resolve_default_model(
                    rec.declaration.default_models.as_deref().unwrap_or(&[]),
                    &chat_ids,
                );
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
                    default_model,
                    default_thinking_level: rec.declaration.default_thinking_level,
                    context_overflow_hint: rec.declaration.context_overflow_hint.clone(),
                    credential: resolved.status,
                });
            }
            providers.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(ProviderListResponse { providers })
        })
    }
}

/// The model to start with, given the provider's ordered preferences and
/// the chat model ids its catalog slice holds right now.
///
/// 1. The first preference the slice holds.
/// 2. Otherwise the slice model sharing the longest id prefix with the first
///    preference, provided they share at least the family (the id up to its
///    first `-`, e.g. `claude`, `gpt`, `codex/gpt`); ties go to the newest
///    by natural (digit-aware) order.
/// 3. Otherwise `None`: consumers keep their old behaviour.
pub fn resolve_default_model(preferences: &[String], slice_ids: &[String]) -> Option<String> {
    if let Some(found) = preferences.iter().find(|p| slice_ids.contains(p)) {
        return Some(found.clone());
    }
    let first = preferences.first()?;
    let family_len = first.find('-').map_or(first.len(), |i| i + 1);
    slice_ids
        .iter()
        .map(|id| (common_prefix_len(first, id), id))
        .filter(|(shared, _)| *shared >= family_len)
        .max_by(|(a_len, a), (b_len, b)| a_len.cmp(b_len).then_with(|| natural_cmp(a, b)))
        .map(|(_, id)| id.clone())
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// Digit runs compare numerically, everything else bytewise, so
/// `glm-5.10` sorts after `glm-5.9` and `gpt-6-sol` after `gpt-5.6-sol`.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let chunks = |s: &str| -> Vec<(u64, String)> {
        let mut out = Vec::new();
        let mut rest = s;
        while !rest.is_empty() {
            let digit = rest.as_bytes()[0].is_ascii_digit();
            let end = rest
                .find(|c: char| c.is_ascii_digit() != digit)
                .unwrap_or(rest.len());
            let (head, tail) = rest.split_at(end);
            out.push(if digit {
                (head.parse().unwrap_or(u64::MAX), String::new())
            } else {
                (0, head.to_string())
            });
            rest = tail;
        }
        out
    };
    chunks(a).cmp(&chunks(b))
}

#[cfg(test)]
mod tests {
    use super::resolve_default_model;
    use crate::types::router::{CredentialOrigin, CredentialStatus, ProviderInfo};
    use serde_json::json;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_model_is_the_first_preference_the_slice_holds() {
        let prefs = ids(&["gpt-6.1-sol", "gpt-6-sol", "gpt-5.6-sol"]);
        let slice = ids(&["gpt-6-astra", "gpt-6-sol", "gpt-5.6-sol"]);
        assert_eq!(
            resolve_default_model(&prefs, &slice).as_deref(),
            Some("gpt-6-sol")
        );
    }

    #[test]
    fn default_model_falls_back_to_the_closest_same_family_model() {
        let prefs = ids(&["claude-sonnet-5-5"]);
        let slice = ids(&[
            "claude-opus-5-5",
            "claude-sonnet-4-6",
            "claude-sonnet-5",
            "claude-haiku-4-5",
        ]);
        // Longest shared prefix wins: `claude-sonnet-5` over `claude-sonnet-4-6`.
        assert_eq!(
            resolve_default_model(&prefs, &slice).as_deref(),
            Some("claude-sonnet-5")
        );
        // Ties go to the newest by natural order, not string order.
        let slice = ids(&["claude-sonnet-4-9", "claude-sonnet-4-10"]);
        assert_eq!(
            resolve_default_model(&prefs, &slice).as_deref(),
            Some("claude-sonnet-4-10")
        );
    }

    #[test]
    fn default_model_is_absent_outside_the_family_or_without_preferences() {
        let prefs = ids(&["claude-sonnet-5-5"]);
        assert_eq!(
            resolve_default_model(&prefs, &ids(&["gpt-6-sol", "o3"])),
            None
        );
        assert_eq!(resolve_default_model(&[], &ids(&["gpt-6-sol"])), None);
        assert_eq!(resolve_default_model(&prefs, &[]), None);
        // A `vendor/` prefix is part of the family.
        let prefs = ids(&["codex/gpt-6.1-sol"]);
        assert_eq!(resolve_default_model(&prefs, &ids(&["gpt-6.1-sol"])), None);
        assert_eq!(
            resolve_default_model(&prefs, &ids(&["codex/gpt-6-sol"])).as_deref(),
            Some("codex/gpt-6-sol")
        );
    }

    #[test]
    fn list_entries_carry_the_defaults_and_old_entries_parse_without_them() {
        let mut entry = info(CredentialStatus::default());
        entry.default_model = Some("claude-sonnet-5-5".into());
        entry.default_thinking_level = Some(crate::types::model::ThinkingLevel::Minimal);
        entry.context_overflow_hint = Some("Raise --ctx-size on the server.".into());
        let wire = serde_json::to_value(&entry).unwrap();
        assert_eq!(wire["default_model"], "claude-sonnet-5-5");
        assert_eq!(wire["default_thinking_level"], "minimal");
        assert_eq!(
            wire["context_overflow_hint"],
            "Raise --ctx-size on the server."
        );
        let bare = serde_json::to_value(info(CredentialStatus::default())).unwrap();
        assert!(bare.get("default_model").is_none());
        assert!(bare.get("default_thinking_level").is_none());
        assert!(bare.get("context_overflow_hint").is_none());
    }

    fn info(credential: CredentialStatus) -> ProviderInfo {
        ProviderInfo {
            id: "anthropic".into(),
            display_name: "Anthropic".into(),
            credential_env_var: Some("ANTHROPIC_API_KEY".into()),
            configured: false,
            available: true,
            supports_model_listing: true,
            icon_svg: None,
            default_model: None,
            default_thinking_level: None,
            context_overflow_hint: None,
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
