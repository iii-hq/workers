//! Durable model catalog: one slice per provider, replaced wholesale on
//! reconcile. Same serialized-writer pattern as the registry: prepare a
//! snapshot, persist it, then publish it in memory while holding the writer
//! lock. Persistence uses the engine's `state::get`/`state::set` iii functions
//! (src/state.rs).
//!
//! Engine-backed coverage: tests/integration.rs (reconcile, restart restore).
use std::collections::HashMap;

use crate::state::{state_get, state_set};
use crate::types::errors::is_function_not_found;
use crate::types::model::Model;
use iii_sdk::{errors::Error, IIIClient};
use tokio::sync::{Mutex, MutexGuard};

const CATALOG_KEY: &str = "catalog";

pub struct CatalogStore {
    iii: IIIClient,
    slices: Mutex<HashMap<String, Vec<Model>>>,
    #[cfg(test)]
    persist_result: Option<Result<(), Error>>,
}

/// A catalog snapshot that is durable but intentionally not visible in
/// memory yet. Provider registration commits it only after the matching
/// registry record is durable; dropping it without `commit` keeps readers on
/// the previous snapshot. Because iii state has no multi-key transaction, a
/// process crash between this durable write and registry commit can still
/// leave a catalog-only snapshot for restart recovery to reconcile.
#[must_use = "a prepared catalog write must be committed or rolled back"]
pub struct PreparedSlice<'a> {
    store: &'a CatalogStore,
    slices: MutexGuard<'a, HashMap<String, Vec<Model>>>,
    previous: HashMap<String, Vec<Model>>,
    next: HashMap<String, Vec<Model>>,
}

impl PreparedSlice<'_> {
    pub fn commit(mut self) {
        *self.slices = self.next;
    }

    /// Restore the durable snapshot while leaving the already-visible memory
    /// snapshot untouched. If this fails, routing still excludes the staged
    /// catalog owner because no registry record was published, but a restart
    /// can reload the orphaned catalog slice until a later reconcile repairs
    /// state.
    pub async fn rollback(self) -> Result<(), Error> {
        self.store.persist(&self.previous).await
    }
}

impl CatalogStore {
    pub fn new(iii: IIIClient) -> Self {
        Self {
            iii,
            slices: Mutex::new(HashMap::new()),
            #[cfg(test)]
            persist_result: None,
        }
    }

    pub async fn load(&self) -> Result<(), Error> {
        // No iii-state worker on this engine (the registry-publish flow boots
        // against a bare `workers: []` engine to collect the interface):
        // start empty. Safe to tolerate exactly this error class — with no
        // state worker, persists can't overwrite the stored snapshot either.
        let stored = match state_get(&self.iii, CATALOG_KEY).await {
            Err(e) if is_function_not_found(&e) => {
                eprintln!("[llm-router] no iii-state worker; catalog starts empty");
                return Ok(());
            }
            other => other?,
        };
        *self.slices.lock().await = serde_json::from_value(stored).unwrap_or_default();
        Ok(())
    }

    pub async fn slice(&self, provider: &str) -> Vec<Model> {
        self.slices
            .lock()
            .await
            .get(provider)
            .cloned()
            .unwrap_or_default()
    }
    pub async fn all(&self) -> Vec<Model> {
        self.slices
            .lock()
            .await
            .values()
            .flatten()
            .cloned()
            .collect()
    }
    pub async fn model_ids(&self) -> Vec<(String, Vec<String>)> {
        self.slices
            .lock()
            .await
            .iter()
            .map(|(p, models)| (p.clone(), models.iter().map(|m| m.id.clone()).collect()))
            .collect()
    }
    pub async fn get(&self, provider: &str, id: &str) -> Option<Model> {
        self.slices
            .lock()
            .await
            .get(provider)
            .and_then(|s| s.iter().find(|m| m.id == id))
            .cloned()
    }

    async fn persist(&self, slices: &HashMap<String, Vec<Model>>) -> Result<(), Error> {
        #[cfg(test)]
        if let Some(result) = &self.persist_result {
            return result.clone();
        }
        let value = serde_json::to_value(slices).unwrap_or_default();
        state_set(&self.iii, CATALOG_KEY, value).await
    }

    /// Persist a candidate slice while blocking catalog readers and writers,
    /// but defer its in-memory publication until `PreparedSlice::commit`.
    pub async fn prepare_slice(
        &self,
        provider: &str,
        models: Vec<Model>,
    ) -> Result<PreparedSlice<'_>, Error> {
        let slices = self.slices.lock().await; // serialized writer
        let previous = slices.clone();
        let mut next = previous.clone();
        next.insert(provider.to_string(), models);
        self.persist(&next).await?;
        Ok(PreparedSlice {
            store: self,
            slices,
            previous,
            next,
        })
    }

    pub async fn set_slice(&self, provider: &str, models: Vec<Model>) -> Result<(), Error> {
        self.prepare_slice(provider, models).await?.commit();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        Model {
            id: id.into(),
            provider: "anthropic".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: None,
            supports_xhigh: None,
            reasoning_efforts: None,
            supports_tools: None,
            supports_vision: None,
            supports_cache: None,
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        }
    }

    fn store_with_persistence(result: Result<(), Error>) -> CatalogStore {
        CatalogStore {
            iii: IIIClient::new("ws://unused.invalid"),
            slices: Mutex::new(HashMap::new()),
            persist_result: Some(result),
        }
    }

    #[tokio::test]
    async fn prepared_slice_is_invisible_until_commit() {
        let store = store_with_persistence(Ok(()));
        store
            .slices
            .lock()
            .await
            .insert("anthropic".into(), vec![model("old")]);

        let prepared = store
            .prepare_slice("anthropic", vec![model("new")])
            .await
            .expect("candidate persists");
        assert_eq!(
            prepared.slices["anthropic"][0].id, "old",
            "persistence alone must not publish the candidate"
        );
        prepared.commit();
        assert_eq!(store.slice("anthropic").await, vec![model("new")]);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_persist_keeps_previous_slice() {
        // An unconnected client makes state::set time out. Paused Tokio time
        // skips the SDK timeout without requiring a live engine.
        let store = CatalogStore::new(IIIClient::new("ws://unconnected.invalid"));
        store
            .slices
            .lock()
            .await
            .insert("anthropic".into(), vec![model("old")]);

        let err = store
            .set_slice("anthropic", vec![model("new")])
            .await
            .expect_err("persistence must fail");
        assert!(matches!(err, Error::Timeout));
        assert_eq!(store.slice("anthropic").await, vec![model("old")]);

        let err = store
            .set_slice("openai", vec![model("gpt")])
            .await
            .expect_err("persistence must fail");
        assert!(matches!(err, Error::Timeout));
        assert!(
            store.slice("openai").await.is_empty(),
            "a failed write must not publish a new provider slice"
        );
    }
}
