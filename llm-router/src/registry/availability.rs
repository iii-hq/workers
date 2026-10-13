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
                    discovery: rec.discovery.clone(),
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
/// the chat model ids its catalog slice holds right now. Candidates are the
/// listed ids, every slice model in the first preference's family (the id
/// up to its first `-`, e.g. `claude`, `gpt`, `codex/gpt`), and every slice
/// model with a ranked variant and the same `vendor/` prefix, so one
/// provider can rank GPT and Claude models in one list.
///
/// 1. Variant first. A variant is the first all-letter id segment after
///    the family (`terra` in `gpt-6.1-terra`, `sonnet` in
///    `claude-sonnet-5-5`); the preferences rank variants in the order they
///    first name them. Within a ranked variant the newest version wins,
///    listed or not, so `gpt-7-luna` never beats `gpt-5.6-terra` when
///    `terra` is listed first.
/// 2. Ids with no ranked variant come last: the first listed one the slice
///    holds, else the one sharing the longest id prefix with the first
///    preference; ties go to the newest by natural (digit-aware) order.
/// 3. No candidate: `None`, and consumers keep their old behaviour.
pub fn resolve_default_model(preferences: &[String], slice_ids: &[String]) -> Option<String> {
    let first = preferences.first()?;
    let family_len = first.find('-').map_or(first.len(), |i| i + 1);
    // `copilot/` in `copilot/gpt-6.1-terra`: a ranked variant from another
    // family still needs it.
    let vendor = &first[..first[..family_len].rfind('/').map_or(0, |i| i + 1)];
    let mut variants: Vec<&str> = Vec::new();
    for v in preferences.iter().filter_map(|p| variant(p)) {
        if !variants.contains(&v) {
            variants.push(v);
        }
    }
    let rank = |id: &str| {
        variant(id)
            .and_then(|v| variants.iter().position(|known| *known == v))
            .unwrap_or(variants.len())
    };
    let listed = |id: &String| {
        preferences
            .iter()
            .position(|p| p == id)
            .unwrap_or(preferences.len())
    };
    // `claude-sonnet-4-6-20260115` → [4, 6]: the numbers around the variant.
    let variant_version = |id: &str| {
        let rest = id.split_once('-').map_or("", |(_, rest)| rest);
        let skip = variant(id).unwrap_or_default();
        version(
            &rest
                .split('-')
                .filter(|s| *s != skip)
                .collect::<Vec<_>>()
                .join("-"),
        )
    };
    slice_ids
        .iter()
        .map(|id| (common_prefix_len(first, id), id))
        .filter(|(shared, id)| {
            *shared >= family_len
                || listed(id) < preferences.len()
                || (id.starts_with(vendor) && rank(id) < variants.len())
        })
        .max_by(|(a_len, a), (b_len, b)| {
            let by_variant = rank(b).cmp(&rank(a));
            let within = if rank(a) < variants.len() {
                variant_version(a).cmp(&variant_version(b))
            } else {
                listed(b)
                    .cmp(&listed(a))
                    .then_with(|| a_len.cmp(b_len))
                    .then_with(|| version(&a[*a_len..]).cmp(&version(&b[*b_len..])))
            };
            by_variant
                .then(within)
                // the plain id over its dated snapshot or a `-mini` variant
                .then_with(|| b.len().cmp(&a.len()))
                .then_with(|| natural_cmp(a, b))
        })
        .map(|(_, id)| id.clone())
}

/// The model variant: the first all-letter segment after the family,
/// `gpt-6-terra` → `terra`, `claude-sonnet-5-5` → `sonnet`. Ids with no
/// such segment (`kimi-k3`, `glm-5.3`) have none.
fn variant(id: &str) -> Option<&str> {
    id.split('-')
        .skip(1)
        .find(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// The version numbers an id starts with after the shared family prefix:
/// `4.1-2025-04-14` → [4, 1], `4o-mini` → [4], `3.5-turbo` → [3, 5]. Stops
/// at the first word or at a date-like run of four or more digits.
fn version(rest: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for part in rest.split(['.', '-']) {
        let digits = part.len() - part.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 || digits > 3 {
            break;
        }
        out.push(part[..digits].parse().unwrap_or(0));
        if digits < part.len() {
            break;
        }
    }
    out
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
    fn terra_outranks_sol_outranks_astra_outranks_luna_then_the_newest_version_wins() {
        let prefs: Vec<String> = ["terra", "sol", "astra", "luna"]
            .iter()
            .flat_map(|v| ["6.1", "6", "5.6"].map(|n| format!("codex/gpt-{n}-{v}")))
            .collect();
        let pick = |slice: &[&str]| resolve_default_model(&prefs, &ids(slice));
        // The Codex catalog that picked `gpt-6-luna` before.
        assert_eq!(
            pick(&[
                "codex/gpt-5.6-luna",
                "codex/gpt-5.6-terra",
                "codex/gpt-6-luna"
            ])
            .as_deref(),
            Some("codex/gpt-5.6-terra")
        );
        // A newer Luna or Sol, listed or not, never beats a Terra.
        assert_eq!(
            pick(&["codex/gpt-7-luna", "codex/gpt-6.1-sol", "codex/gpt-5-terra"]).as_deref(),
            Some("codex/gpt-5-terra")
        );
        // No Terra: Sol over Astra and Luna.
        assert_eq!(
            pick(&[
                "codex/gpt-7-luna",
                "codex/gpt-6.1-astra",
                "codex/gpt-5.6-sol"
            ])
            .as_deref(),
            Some("codex/gpt-5.6-sol")
        );
        // No Terra or Sol: Astra over a newer Luna.
        assert_eq!(
            pick(&[
                "codex/gpt-7-luna",
                "codex/gpt-6-astra",
                "codex/gpt-5.6-luna"
            ])
            .as_deref(),
            Some("codex/gpt-6-astra")
        );
        // Within a variant the newest version wins, listed or not.
        assert_eq!(
            pick(&[
                "codex/gpt-6.1-terra",
                "codex/gpt-7-terra",
                "codex/gpt-6.2-terra"
            ])
            .as_deref(),
            Some("codex/gpt-7-terra")
        );
        // Unranked ids come after every ranked variant.
        assert_eq!(
            pick(&["codex/gpt-7", "codex/gpt-7-mini", "codex/gpt-5.6-luna"]).as_deref(),
            Some("codex/gpt-5.6-luna")
        );
        assert_eq!(
            pick(&["codex/gpt-7-mini", "codex/gpt-7"]).as_deref(),
            Some("codex/gpt-7")
        );
    }

    #[test]
    fn sonnet_outranks_opus_fable_haiku_then_the_newest_version_wins() {
        let prefs = ids(&[
            "claude-sonnet-5-5",
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-haiku-4-5",
        ]);
        let pick = |slice: &[&str]| resolve_default_model(&prefs, &ids(slice));
        // A newer Opus, Fable or Haiku never beats a Sonnet.
        assert_eq!(
            pick(&[
                "claude-opus-6",
                "claude-haiku-6",
                "claude-sonnet-4-6-20260115"
            ])
            .as_deref(),
            Some("claude-sonnet-4-6-20260115")
        );
        // No Sonnet: Opus, then Fable, then Haiku.
        assert_eq!(
            pick(&[
                "claude-haiku-5-5",
                "claude-fable-5-1",
                "claude-opus-4-1-20250805"
            ])
            .as_deref(),
            Some("claude-opus-4-1-20250805")
        );
        assert_eq!(
            pick(&["claude-haiku-5-5", "claude-fable-5-1"]).as_deref(),
            Some("claude-fable-5-1")
        );
        // Within a variant the newest version wins, then the plain id over
        // its dated snapshot.
        assert_eq!(
            pick(&[
                "claude-sonnet-4-6",
                "claude-sonnet-5-5-20260901",
                "claude-sonnet-5-5",
                "claude-sonnet-5",
            ])
            .as_deref(),
            Some("claude-sonnet-5-5")
        );
        // The old `claude-<version>-<variant>` naming ranks the same way.
        assert_eq!(
            pick(&["claude-3-opus-20240229", "claude-3-5-sonnet-20241022"]).as_deref(),
            Some("claude-3-5-sonnet-20241022")
        );
    }

    #[test]
    fn one_list_ranks_gpt_then_claude_variants() {
        // Copilot serves both families: GPT variants first, then Claude.
        let prefs = ids(&[
            "copilot/gpt-6.1-terra",
            "copilot/gpt-6.1-sol",
            "copilot/gpt-6.1-luna",
            "copilot/claude-sonnet-5.5",
            "copilot/claude-opus-5.5",
            "copilot/claude-fable-5.1",
            "copilot/claude-haiku-5.5",
        ]);
        let pick = |slice: &[&str]| resolve_default_model(&prefs, &ids(slice));
        assert_eq!(
            pick(&["copilot/claude-sonnet-5.5", "copilot/gpt-5-luna"]).as_deref(),
            Some("copilot/gpt-5-luna")
        );
        // No ranked GPT: unlisted Claude versions rank by variant, then version.
        assert_eq!(
            pick(&[
                "copilot/gpt-4.1",
                "copilot/claude-haiku-4.5",
                "copilot/claude-opus-4.5",
                "copilot/claude-sonnet-4.5",
                "copilot/claude-sonnet-4.6",
            ])
            .as_deref(),
            Some("copilot/claude-sonnet-4.6")
        );
        assert_eq!(
            pick(&[
                "copilot/gpt-4.1",
                "copilot/claude-haiku-4.5",
                "copilot/claude-opus-4.5"
            ])
            .as_deref(),
            Some("copilot/claude-opus-4.5")
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
    fn family_ties_go_to_the_highest_version_then_the_plain_id() {
        // A Copilot account whose catalog has none of the preferred models:
        // every id shares only `copilot/gpt-`. The newest version wins, and a
        // plain id wins over its dated snapshot or a `-mini` variant.
        let prefs = ids(&["copilot/gpt-6.1-sol", "copilot/gpt-6-sol"]);
        let slice = ids(&[
            "copilot/gpt-4o-mini-2024-07-18",
            "copilot/gpt-4o-2024-11-20",
            "copilot/gpt-4.1-2025-04-14",
            "copilot/gpt-3.5-turbo-0613",
            "copilot/gpt-4-o-preview",
            "copilot/gpt-4.1",
            "copilot/gpt-4o-mini",
            "copilot/gpt-4o",
        ]);
        assert_eq!(
            resolve_default_model(&prefs, &slice).as_deref(),
            Some("copilot/gpt-4.1")
        );
        let slice = ids(&[
            "copilot/gpt-4o-mini",
            "copilot/gpt-4o-2024-11-20",
            "copilot/gpt-4o",
        ]);
        assert_eq!(
            resolve_default_model(&prefs, &slice).as_deref(),
            Some("copilot/gpt-4o")
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
            discovery: None,
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
