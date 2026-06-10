//! models::list / get / supports semantics (spec § Capability strings).
use crate::types::model::Model;

use super::store::CatalogStore;

/// Capability string → Model flag. Unknown strings match nothing.
pub fn model_supports(model: &Model, capability: &str) -> bool {
    match capability {
        "tools" => model.supports_tools == Some(true),
        "vision" => model.supports_vision == Some(true),
        "cache" => model.supports_cache == Some(true),
        "structured_output" => model.supports_structured_output == Some(true),
        "thinking" | "thinking:low" | "thinking:medium" | "thinking:high" => {
            model.supports_thinking == Some(true)
        }
        "thinking:xhigh" => model.supports_xhigh == Some(true),
        _ => false,
    }
}

pub async fn models_list(
    store: &CatalogStore,
    provider: Option<&str>,
    capability: Option<&str>,
) -> Vec<Model> {
    let mut models = match provider {
        Some(p) => store.slice(p).await,
        None => store.all().await,
    };
    if let Some(cap) = capability {
        models.retain(|m| model_supports(m, cap));
    }
    models
}

pub async fn models_get(store: &CatalogStore, provider: &str, id: &str) -> Option<Model> {
    store.get(provider, id).await
}

/// Unknown model → false; request-shaping callers use models::get → null for
/// the fail-open cold-window rule (spec § Capability defaults).
pub async fn models_supports(
    store: &CatalogStore,
    provider: &str,
    id: &str,
    capability: &str,
) -> bool {
    match store.get(provider, id).await {
        Some(m) => model_supports(&m, capability),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::model::Model;

    fn sonnet() -> Model {
        Model {
            id: "claude-sonnet-4".into(),
            provider: "anthropic".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: Some(false),
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: None,
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        }
    }

    // Store-backed list/get/supports flows are exercised against a real engine
    // in tests/integration.rs; the capability mapping is pure and pinned here.
    #[test]
    fn capability_strings_map_to_model_flags() {
        let m = sonnet();
        assert!(model_supports(&m, "tools"));
        assert!(model_supports(&m, "vision"));
        assert!(model_supports(&m, "thinking"));
        assert!(model_supports(&m, "thinking:medium"));
        assert!(!model_supports(&m, "thinking:xhigh"));
        assert!(!model_supports(&m, "cache")); // absent flag reads as false
        assert!(!model_supports(&m, "structured_output"));
        assert!(!model_supports(&m, "bogus")); // unknown capability strings match nothing
    }
}
