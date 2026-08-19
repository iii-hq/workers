//! decide(): ordered candidate list (spec § Routing). MVP consumes
//! candidates[0]; the list shape is the future fallback seam.
//! `router::route` exposes the same decision as a read-only preview so
//! consumers that need the provider before streaming (prompt selection,
//! provisioning metadata) can pin it as the explicit `provider` on
//! `router::chat` — preview and execution can never diverge.
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::catalog::queries::effective_model_ref;
use crate::catalog::store::CatalogStore;
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::store::RegistryStore;
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{RouteRequest, RouteResponse};

#[derive(Debug, Clone, PartialEq)]
pub struct Heuristic {
    pub pattern: String,
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct DecideInput {
    pub model: String,
    pub provider: Option<String>,
    pub registered_providers: Vec<String>,
    pub available_providers: Vec<String>,
    pub catalog: Vec<(String, Vec<String>)>, // provider id -> model ids
    pub heuristics: Vec<Heuristic>,
    pub default_provider: Option<String>,
}

pub fn decide(input: &DecideInput) -> Result<Vec<String>, RouterError> {
    let registered = |id: &str| input.registered_providers.iter().any(|p| p == id);
    let available = |id: &str| input.available_providers.iter().any(|p| p == id);

    // 0. Composite `provider::model` ids (the console's display form) resolve
    // in models::get/budget — dispatch must agree instead of shipping the
    // literal string to the default provider. Split when the prefix names a
    // registered or catalog provider, so the composite routes exactly like the
    // explicit pair; unknown prefixes stay literal (`::` is legal in an id).
    let (provider, model) = effective_model_ref(input.provider.as_deref(), &input.model, |p| {
        registered(p) || input.catalog.iter().any(|(owner, _)| owner == p)
    });

    // 1. Explicit provider — sole candidate; cold-catalog tolerant; typos loud.
    if let Some(provider) = provider {
        if !registered(provider) {
            return Err(RouterError::new(
                RouterCode::UnknownProvider,
                format!("unknown provider {provider}"),
            ));
        }
        if !available(provider) {
            return Err(RouterError::new(
                RouterCode::ProviderUnavailable,
                format!("provider {provider} unavailable"),
            ));
        }
        return Ok(vec![provider.to_string()]);
    }

    // 2. Unique available catalog owner; 2+ available owners → ambiguous (the
    // router never guesses). Ignore stale catalog slices belonging to a
    // missing/down provider so route previews cannot select a known-dead
    // dispatch target.
    let owners: Vec<&str> = input
        .catalog
        .iter()
        .filter(|(provider, ids)| registered(provider) && ids.iter().any(|m| m == model))
        .map(|(p, _)| p.as_str())
        .collect();
    let mut available_owners: Vec<&str> = owners
        .iter()
        .copied()
        .filter(|provider| available(provider))
        .collect();
    available_owners.sort_unstable();
    match available_owners.len() {
        1 => return Ok(vec![available_owners[0].to_string()]),
        n if n > 1 => {
            return Err(RouterError::new(
                RouterCode::AmbiguousModel,
                format!(
                    "ambiguous model {model} (providers: {})",
                    available_owners.join(", ")
                ),
            ))
        }
        _ => {}
    }
    let mut unavailable_matches: Vec<&str> = owners
        .iter()
        .copied()
        .filter(|provider| !available(provider))
        .collect();

    // 3. Operator heuristics from the llm-router entry; first available match wins.
    for h in &input.heuristics {
        if !registered(&h.provider) {
            continue;
        }
        let Ok(re) = regex::Regex::new(&h.pattern) else {
            continue; // an invalid operator regex never takes the router down
        };
        if re.is_match(model) {
            if available(&h.provider) {
                return Ok(vec![h.provider.clone()]);
            }
            unavailable_matches.push(&h.provider);
        }
    }

    // 4. Configured default provider makes routing a total function.
    if let Some(default) = &input.default_provider {
        if registered(default) && available(default) {
            return Ok(vec![default.clone()]);
        }
        if registered(default) {
            unavailable_matches.push(default);
        }
    }

    // 5. Preserve an actionable distinction between an unknown model and a
    // model whose catalog, heuristic, or default candidates are all down.
    unavailable_matches.sort_unstable();
    unavailable_matches.dedup();
    if !unavailable_matches.is_empty() {
        return Err(RouterError::new(
            RouterCode::ProviderUnavailable,
            format!(
                "no available provider for model {model} (unavailable: {})",
                unavailable_matches.join(", ")
            ),
        ));
    }

    // 6. Loud failure.
    Err(RouterError::new(
        RouterCode::NoProviderForModel,
        format!("no provider registered for model {model}"),
    ))
}

/// The `router::route` iii function: `{model, provider?}` →
/// `{provider, candidates}`. Same inputs, same `decide()`, same error codes
/// as the chat pipeline's routing step — just without the stream.
pub fn make_route(
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    config: ConfigCell,
) -> impl Fn(RouteRequest) -> BoxFuture<'static, Result<RouteResponse, Error>> + Send + Sync + 'static
{
    move |req: RouteRequest| {
        let (registry, catalog, config) = (registry.clone(), catalog.clone(), config.clone());
        Box::pin(async move {
            if req.model.is_empty() {
                return Err(
                    RouterError::new(RouterCode::InvalidRequest, "model is required").into(),
                );
            }
            let config = snapshot(&config);
            let heuristics = config.settings().routing_heuristics.clone();
            let default_provider = config.settings().default_provider.clone();
            let providers = registry.list().await;
            let candidates = decide(&DecideInput {
                model: req.model,
                provider: req.provider,
                registered_providers: providers
                    .iter()
                    .map(|record| record.declaration.id.clone())
                    .collect(),
                available_providers: providers
                    .iter()
                    .filter(|record| record.available)
                    .map(|record| record.declaration.id.clone())
                    .collect(),
                catalog: catalog.model_ids().await,
                heuristics,
                default_provider,
            })
            .map_err(Error::from)?;
            Ok(RouteResponse {
                provider: candidates[0].clone(),
                candidates,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::errors::RouterCode;

    fn base() -> DecideInput {
        DecideInput {
            model: String::new(),
            provider: None,
            registered_providers: vec!["anthropic".into(), "openai".into(), "lmstudio".into()],
            available_providers: vec!["anthropic".into(), "openai".into(), "lmstudio".into()],
            catalog: vec![
                ("anthropic".into(), vec!["claude-sonnet-4".into()]),
                ("openai".into(), vec!["gpt-5".into(), "shared-model".into()]),
                (
                    "lmstudio".into(),
                    vec!["local-llama".into(), "shared-model".into()],
                ),
            ],
            heuristics: vec![Heuristic {
                pattern: "^gpt-".into(),
                provider: "openai".into(),
            }],
            default_provider: None,
        }
    }

    #[test]
    fn step1_explicit_provider_wins_even_on_cold_catalog_and_typos_fail_loudly() {
        let mut input = base();
        input.model = "brand-new".into();
        input.provider = Some("anthropic".into());
        assert_eq!(decide(&input).unwrap(), vec!["anthropic"]);
        input.provider = Some("anthropc".into());
        assert_eq!(
            decide(&input).unwrap_err().code,
            RouterCode::UnknownProvider
        );
    }

    #[test]
    fn step2_unique_catalog_owner_routes_implicitly_ambiguity_never_guesses() {
        let mut input = base();
        input.model = "local-llama".into();
        assert_eq!(decide(&input).unwrap(), vec!["lmstudio"]);
        input.model = "shared-model".into();
        let err = decide(&input).unwrap_err();
        assert_eq!(err.code, RouterCode::AmbiguousModel);
        assert_eq!(
            err.message,
            "ambiguous model shared-model (providers: lmstudio, openai)"
        );
    }

    #[test]
    fn step3_heuristics_first_match_registered_only_invalid_regex_skipped() {
        let mut input = base();
        input.model = "gpt-99".into();
        assert_eq!(decide(&input).unwrap(), vec!["openai"]);
        input.heuristics = vec![
            Heuristic {
                pattern: "([".into(),
                provider: "openai".into(),
            }, // invalid: skipped
            Heuristic {
                pattern: "^gpt-".into(),
                provider: "unregistered".into(),
            }, // skipped
            Heuristic {
                pattern: "^gpt-".into(),
                provider: "openai".into(),
            },
        ];
        assert_eq!(decide(&input).unwrap(), vec!["openai"]);
    }

    // Composite `provider::model` ids (the console's display form) resolve in
    // models::get/budget — dispatch must agree. A known prefix routes exactly
    // like the explicit pair; a `::` id whose prefix names no provider stays
    // literal and behaves as it always did.
    #[test]
    fn step0_composite_model_ids_route_like_the_explicit_pair() {
        let mut input = base();
        // Registered prefix → step 1, even when the split model isn't
        // cataloged yet (cold-catalog tolerant, like the explicit pair).
        input.model = "anthropic::brand-new".into();
        assert_eq!(decide(&input).unwrap(), vec!["anthropic"]);
        // The harness's shape: explicit provider agreeing with the prefix.
        input.model = "anthropic::claude-sonnet-4".into();
        input.provider = Some("anthropic".into());
        assert_eq!(decide(&input).unwrap(), vec!["anthropic"]);
        // Contradiction → no split; the explicit provider wins, as before.
        input.provider = Some("openai".into());
        assert_eq!(decide(&input).unwrap(), vec!["openai"]);
        // Non-provider prefix stays literal: no owner, no default → the same
        // loud error as before, never a guessed split.
        input.provider = None;
        input.model = "weird::thing".into();
        assert_eq!(
            decide(&input).unwrap_err().code,
            RouterCode::NoProviderForModel
        );
        // Catalog-owner prefix whose provider lost registration: the loud
        // unknown-provider the explicit pair would get — not a silent
        // default-provider dispatch of the literal composite.
        input.model = "lmstudio::local-llama".into();
        input.default_provider = Some("anthropic".into());
        input.registered_providers.retain(|p| p != "lmstudio");
        assert_eq!(
            decide(&input).unwrap_err().code,
            RouterCode::UnknownProvider
        );
    }

    #[test]
    fn step4_default_provider_makes_routing_total_step5_throws_otherwise() {
        let mut input = base();
        input.model = "mystery".into();
        assert_eq!(
            decide(&input).unwrap_err().code,
            RouterCode::NoProviderForModel
        );
        input.default_provider = Some("anthropic".into());
        assert_eq!(decide(&input).unwrap(), vec!["anthropic"]);
    }

    #[test]
    fn explicit_unavailable_provider_is_distinct_from_unknown_provider() {
        let mut input = base();
        input.model = "gpt-5".into();
        input.provider = Some("openai".into());
        input.available_providers.retain(|id| id != "openai");

        let err = decide(&input).unwrap_err();
        assert_eq!(err.code, RouterCode::ProviderUnavailable);
        assert_eq!(err.message, "provider openai unavailable");
    }

    #[test]
    fn stale_catalog_owner_is_excluded_from_routing() {
        let mut input = base();
        input.model = "shared-model".into();
        input.available_providers.retain(|id| id != "lmstudio");

        assert_eq!(decide(&input).unwrap(), vec!["openai"]);

        input.available_providers.retain(|id| id != "openai");
        let err = decide(&input).unwrap_err();
        assert_eq!(err.code, RouterCode::ProviderUnavailable);
        assert_eq!(
            err.message,
            "no available provider for model shared-model (unavailable: lmstudio, openai)"
        );
    }

    #[test]
    fn stale_owner_does_not_block_an_available_default() {
        let mut input = base();
        input.model = "local-llama".into();
        input.available_providers.retain(|id| id != "lmstudio");
        input.default_provider = Some("anthropic".into());

        assert_eq!(decide(&input).unwrap(), vec!["anthropic"]);
    }

    #[test]
    fn unavailable_heuristic_or_default_reports_provider_unavailable() {
        let mut input = base();
        input.model = "gpt-future".into();
        input.available_providers.retain(|id| id != "openai");

        let err = decide(&input).unwrap_err();
        assert_eq!(err.code, RouterCode::ProviderUnavailable);
        assert!(err.message.contains("unavailable: openai"));

        input.model = "mystery".into();
        input.heuristics.clear();
        input.default_provider = Some("openai".into());
        let err = decide(&input).unwrap_err();
        assert_eq!(err.code, RouterCode::ProviderUnavailable);
        assert!(err.message.contains("unavailable: openai"));
    }
}
