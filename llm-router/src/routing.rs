//! decide(): ordered candidate list (spec § Routing). MVP consumes
//! candidates[0]; the list shape is the future fallback seam.
//! `router::route` exposes the same decision as a read-only preview so
//! consumers that need the provider before streaming (prompt selection,
//! provisioning metadata) can pin it as the explicit `provider` on
//! `router::chat` — preview and execution can never diverge.
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

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

    // 1. Explicit provider — sole candidate; cold-catalog tolerant; typos loud.
    if let Some(provider) = &input.provider {
        if !registered(provider) {
            return Err(RouterError::new(
                RouterCode::UnknownProvider,
                format!(
                    "Provider \"{provider}\" is not registered. Choose a configured provider and try again."
                ),
            ));
        }
        if !available(provider) {
            return Err(RouterError::new(
                RouterCode::ProviderUnavailable,
                format!(
                    "Provider \"{provider}\" is temporarily unavailable. Try again or choose another provider."
                ),
            ));
        }
        return Ok(vec![provider.clone()]);
    }

    // 2. Unique available catalog owner; 2+ available owners → ambiguous (the
    // router never guesses). Ignore stale catalog slices belonging to a
    // missing/down provider so route previews cannot select a known-dead
    // dispatch target.
    let owners: Vec<&str> = input
        .catalog
        .iter()
        .filter(|(provider, ids)| registered(provider) && ids.iter().any(|m| m == &input.model))
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
                    "Model \"{}\" is available from multiple providers: {}. Choose a provider and try again.",
                    input.model,
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
        if re.is_match(&input.model) {
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
                "No provider currently available can serve model \"{}\" (unavailable: {}). Try again later or choose another model.",
                input.model,
                unavailable_matches.join(", ")
            ),
        ));
    }

    // 6. Loud failure.
    Err(RouterError::new(
        RouterCode::NoProviderForModel,
        format!(
            "No configured provider can serve model \"{}\". Choose another model or configure a compatible provider.",
            input.model
        ),
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
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "Select a model before sending the request.",
                )
                .into());
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
            "Model \"shared-model\" is available from multiple providers: lmstudio, openai. Choose a provider and try again."
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
        assert_eq!(
            err.message,
            "Provider \"openai\" is temporarily unavailable. Try again or choose another provider."
        );
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
            "No provider currently available can serve model \"shared-model\" (unavailable: lmstudio, openai). Try again later or choose another model."
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
