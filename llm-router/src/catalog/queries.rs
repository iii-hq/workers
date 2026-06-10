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
    use crate::catalog::store::CatalogStore;
    use crate::testkit::fake_bus::FakeBus;
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

    #[tokio::test]
    async fn slices_replace_wholesale_and_survive_restart() {
        let bus = FakeBus::new();
        let store = CatalogStore::new(bus.clone());
        store.load().await.unwrap();
        store.set_slice("anthropic", vec![sonnet()]).await.unwrap();
        store.set_slice("anthropic", vec![]).await.unwrap(); // reconcile-to-empty = eviction
        let store2 = CatalogStore::new(bus);
        store2.load().await.unwrap();
        assert!(store2.slice("anthropic").await.is_empty());
    }

    #[tokio::test]
    async fn queries_filter_and_map_capability_strings() {
        let bus = FakeBus::new();
        let store = CatalogStore::new(bus);
        store.load().await.unwrap();
        store.set_slice("anthropic", vec![sonnet()]).await.unwrap();

        assert_eq!(models_list(&store, Some("anthropic"), None).await.len(), 1);
        assert_eq!(models_list(&store, None, Some("tools")).await.len(), 1);
        assert_eq!(
            models_list(&store, None, Some("structured_output"))
                .await
                .len(),
            0
        );
        assert!(models_get(&store, "anthropic", "claude-sonnet-4")
            .await
            .is_some());
        assert!(models_get(&store, "anthropic", "nope").await.is_none()); // the cold-window signal

        let sup = |cap: &str| {
            let store = &store;
            let cap = cap.to_string();
            async move { models_supports(store, "anthropic", "claude-sonnet-4", &cap).await }
        };
        assert!(sup("tools").await);
        assert!(sup("thinking:medium").await);
        assert!(!sup("thinking:xhigh").await);
        assert!(!sup("cache").await); // absent flag on a known model reads as false
        assert!(!sup("bogus").await); // unknown capability strings match nothing
        assert!(!models_supports(&store, "anthropic", "nope", "tools").await); // unknown model
    }
}
