//! decide(): ordered candidate list (spec § Routing). MVP consumes
//! candidates[0]; the list shape is the future fallback seam.
//! `router::route` exposes the same decision as a read-only preview so
//! consumers that need the provider before streaming (prompt selection,
//! provisioning metadata) can pin it as the explicit `provider` on
//! `router::chat` — preview and execution can never diverge.
use std::sync::{Arc, RwLock};

use futures::future::BoxFuture;
use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::catalog::store::CatalogStore;
use crate::registry::store::RegistryStore;
use crate::settings::RouterSettings;
use crate::types::errors::{RouterCode, RouterError};

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
    pub catalog: Vec<(String, Vec<String>)>, // provider id -> model ids
    pub heuristics: Vec<Heuristic>,
    pub default_provider: Option<String>,
}

pub fn decide(input: &DecideInput) -> Result<Vec<String>, RouterError> {
    let registered = |id: &str| input.registered_providers.iter().any(|p| p == id);

    // 1. Explicit provider — sole candidate; cold-catalog tolerant; typos loud.
    if let Some(provider) = &input.provider {
        if !registered(provider) {
            return Err(RouterError::new(
                RouterCode::UnknownProvider,
                format!("unknown provider {provider}"),
            ));
        }
        return Ok(vec![provider.clone()]);
    }

    // 2. Unique catalog owner; 2+ owners → ambiguous (the router never guesses).
    let mut owners: Vec<&str> = input
        .catalog
        .iter()
        .filter(|(_, ids)| ids.iter().any(|m| m == &input.model))
        .map(|(p, _)| p.as_str())
        .collect();
    owners.sort_unstable();
    match owners.len() {
        1 => return Ok(vec![owners[0].to_string()]),
        n if n > 1 => {
            return Err(RouterError::new(
                RouterCode::AmbiguousModel,
                format!(
                    "ambiguous model {} (providers: {})",
                    input.model,
                    owners.join(", ")
                ),
            ))
        }
        _ => {}
    }

    // 3. Operator heuristics from the llm-router entry; first match wins.
    for h in &input.heuristics {
        if !registered(&h.provider) {
            continue;
        }
        let Ok(re) = regex::Regex::new(&h.pattern) else {
            continue; // an invalid operator regex never takes the router down
        };
        if re.is_match(&input.model) {
            return Ok(vec![h.provider.clone()]);
        }
    }

    // 4. Configured default provider makes routing a total function.
    if let Some(default) = &input.default_provider {
        if registered(default) {
            return Ok(vec![default.clone()]);
        }
    }

    // 5. Loud failure.
    Err(RouterError::new(
        RouterCode::NoProviderForModel,
        format!("no provider registered for model {}", input.model),
    ))
}

/// The `router::route` iii function: `{model, provider?}` →
/// `{provider, candidates}`. Same inputs, same `decide()`, same error codes
/// as the chat pipeline's routing step — just without the stream.
pub fn make_route(
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    settings: Arc<RwLock<RouterSettings>>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (registry, catalog, settings) = (registry.clone(), catalog.clone(), settings.clone());
        Box::pin(async move {
            let model = raw
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if model.is_empty() {
                return Err(
                    RouterError::new(RouterCode::InvalidRequest, "model is required").into(),
                );
            }
            let provider = raw
                .get("provider")
                .and_then(Value::as_str)
                .map(String::from);
            let (heuristics, default_provider) = {
                let s = settings.read().unwrap();
                (s.routing_heuristics.clone(), s.default_provider.clone())
            };
            let candidates = decide(&DecideInput {
                model,
                provider,
                registered_providers: registry.ids().await,
                catalog: catalog.model_ids().await,
                heuristics,
                default_provider,
            })
            .map_err(IIIError::from)?;
            Ok(json!({ "provider": candidates[0], "candidates": candidates }))
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
}
