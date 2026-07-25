//! TTL cache over a [`ModelResolver`]. Model budgets change only when a
//! provider reconciles its catalog slice or an operator edits limits, yet
//! the resolver was hitting `router::models::budget` on every non-inline
//! assemble/compact/count call. The decorator bounds that to one bus round
//! trip per (provider, id) per TTL window, and [`CachingModelResolver::flush`]
//! empties it eagerly when the router announces `router::models::changed`.
//!
//! TTLs: known budgets are safe to hold for a minute (staleness only skews
//! an estimate reserve); an unknown model (`Ok(None)`) is held briefly so a
//! just-reconciled catalog is picked up quickly; errors are never cached —
//! an absent router must not linger after it comes up.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ports::{Clock, ModelBudget, ModelResolver};

const MODELS_CHANGED_FN_ID: &str = "context::on-models-changed";

const HIT_TTL_MS: i64 = 60_000;
const NEGATIVE_TTL_MS: i64 = 5_000;

struct CacheEntry {
    budget: Option<ModelBudget>,
    stored_at_ms: i64,
}

pub struct CachingModelResolver {
    inner: Arc<dyn ModelResolver>,
    clock: Arc<dyn Clock>,
    entries: RwLock<HashMap<(Option<String>, String), CacheEntry>>,
}

impl CachingModelResolver {
    pub fn new(inner: Arc<dyn ModelResolver>, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner,
            clock,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Drop every cached budget (bound to `router::models::changed`).
    pub fn flush(&self) {
        self.entries
            .write()
            .expect("model-budget cache lock poisoned")
            .clear();
    }

    fn fresh(&self, key: &(Option<String>, String)) -> Option<Option<ModelBudget>> {
        let entries = self
            .entries
            .read()
            .expect("model-budget cache lock poisoned");
        let entry = entries.get(key)?;
        let ttl = if entry.budget.is_some() {
            HIT_TTL_MS
        } else {
            NEGATIVE_TTL_MS
        };
        (self.clock.now_ms().saturating_sub(entry.stored_at_ms) <= ttl)
            .then(|| entry.budget.clone())
    }
}

#[async_trait]
impl ModelResolver for CachingModelResolver {
    async fn get_model_budget(
        &self,
        provider: Option<&str>,
        id: &str,
    ) -> Result<Option<ModelBudget>, String> {
        let key = (provider.map(str::to_string), id.to_string());
        if let Some(budget) = self.fresh(&key) {
            return Ok(budget);
        }
        // The lock is never held across this await; concurrent misses may
        // duplicate one resolve, which is harmless and self-heals on store.
        let budget = self.inner.get_model_budget(provider, id).await?;
        self.entries
            .write()
            .expect("model-budget cache lock poisoned")
            .insert(
                key,
                CacheEntry {
                    budget: budget.clone(),
                    stored_at_ms: self.clock.now_ms(),
                },
            );
        Ok(budget)
    }
}

/// `router::models::changed` payload — `{ provider, count }`. The handler
/// flushes everything regardless, so the fields are advisory only.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
struct OnModelsChangedEvent {
    #[serde(default)]
    #[allow(dead_code)]
    provider: Option<String>,
}

/// Ack returned by the internal `context::on-models-changed` handler.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct OnModelsChangedResponse {
    ok: bool,
}

/// Best-effort flush binding: when the router announces a catalog change
/// (`router::models::changed`), drop every cached budget so the next call
/// re-resolves. A failed registration (router absent at boot, older router
/// without the trigger type) only costs TTL-bounded staleness — never boot.
pub fn register_models_changed_flush(iii: &IIIClient, cache: Arc<CachingModelResolver>) {
    iii.register_function(
        MODELS_CHANGED_FN_ID,
        RegisterFunction::new_async(move |_event: OnModelsChangedEvent| {
            let cache = cache.clone();
            async move {
                cache.flush();
                Ok::<OnModelsChangedResponse, iii_sdk::errors::Error>(OnModelsChangedResponse {
                    ok: true,
                })
            }
        })
        .description("Internal: flush the model-budget cache when the router's catalog changes.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    if let Err(e) = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "router::models::changed".to_string(),
        function_id: MODELS_CHANGED_FN_ID.to_string(),
        config: json!({}),
        metadata: None,
        namespace: iii.namespace(),
    }) {
        tracing::warn!(
            error = %e,
            "could not bind router::models::changed; model-budget cache degrades to TTL-only invalidation"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Model;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    struct FakeClock(AtomicI64);
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct CountingResolver {
        calls: AtomicUsize,
        response: Result<Option<ModelBudget>, String>,
    }

    #[async_trait]
    impl ModelResolver for CountingResolver {
        async fn get_model_budget(
            &self,
            _provider: Option<&str>,
            _id: &str,
        ) -> Result<Option<ModelBudget>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.response.clone()
        }
    }

    fn budget() -> ModelBudget {
        let model: Model = serde_json::from_value(serde_json::json!({
            "id": "m",
            "provider": "p",
            "context_window": 100_000,
            "max_output_tokens": 8_000,
        }))
        .unwrap();
        ModelBudget {
            effective_max_output_tokens: model.max_output_tokens,
            model,
        }
    }

    fn harness(
        response: Result<Option<ModelBudget>, String>,
    ) -> (Arc<CountingResolver>, Arc<FakeClock>, CachingModelResolver) {
        let inner = Arc::new(CountingResolver {
            calls: AtomicUsize::new(0),
            response,
        });
        let clock = Arc::new(FakeClock(AtomicI64::new(0)));
        let cache = CachingModelResolver::new(inner.clone(), clock.clone());
        (inner, clock, cache)
    }

    #[tokio::test]
    async fn hit_within_ttl_avoids_the_inner_call() {
        let (inner, clock, cache) = harness(Ok(Some(budget())));
        assert!(cache
            .get_model_budget(Some("p"), "m")
            .await
            .unwrap()
            .is_some());
        clock.0.store(HIT_TTL_MS, Ordering::SeqCst); // exactly at the TTL edge
        assert!(cache
            .get_model_budget(Some("p"), "m")
            .await
            .unwrap()
            .is_some());
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn expiry_re_resolves() {
        let (inner, clock, cache) = harness(Ok(Some(budget())));
        cache.get_model_budget(Some("p"), "m").await.unwrap();
        clock.0.store(HIT_TTL_MS + 1, Ordering::SeqCst);
        cache.get_model_budget(Some("p"), "m").await.unwrap();
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn keys_are_per_provider_and_id() {
        let (inner, _clock, cache) = harness(Ok(Some(budget())));
        cache.get_model_budget(Some("p"), "m").await.unwrap();
        cache.get_model_budget(Some("q"), "m").await.unwrap();
        cache.get_model_budget(None, "m").await.unwrap();
        assert_eq!(inner.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn unknown_model_uses_the_short_negative_ttl() {
        let (inner, clock, cache) = harness(Ok(None));
        assert!(cache
            .get_model_budget(Some("p"), "m")
            .await
            .unwrap()
            .is_none());
        clock.0.store(NEGATIVE_TTL_MS + 1, Ordering::SeqCst);
        assert!(cache
            .get_model_budget(Some("p"), "m")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            inner.calls.load(Ordering::SeqCst),
            2,
            "a just-reconciled model must be seen within seconds"
        );
    }

    #[tokio::test]
    async fn errors_are_never_cached() {
        let (inner, _clock, cache) = harness(Err("router unreachable".into()));
        assert!(cache.get_model_budget(Some("p"), "m").await.is_err());
        assert!(cache.get_model_budget(Some("p"), "m").await.is_err());
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn flush_clears_every_entry() {
        let (inner, _clock, cache) = harness(Ok(Some(budget())));
        cache.get_model_budget(Some("p"), "m").await.unwrap();
        cache.flush();
        cache.get_model_budget(Some("p"), "m").await.unwrap();
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }
}
