//! Durable model catalog: one slice per provider, replaced wholesale on
//! reconcile. Same serialized-writer pattern as the registry — persisted via
//! the engine's `state::get`/`state::set` iii functions (src/state.rs).
//!
//! Engine-backed coverage: tests/integration.rs (reconcile, restart restore).
use std::collections::HashMap;

use crate::state::{state_get, state_set};
use crate::types::model::Model;
use iii_sdk::{IIIError, III};
use tokio::sync::Mutex;

const CATALOG_KEY: &str = "catalog";

pub struct CatalogStore {
    iii: III,
    slices: Mutex<HashMap<String, Vec<Model>>>,
}

impl CatalogStore {
    pub fn new(iii: III) -> Self {
        Self {
            iii,
            slices: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Result<(), IIIError> {
        let stored = state_get(&self.iii, CATALOG_KEY).await?;
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

    pub async fn set_slice(&self, provider: &str, models: Vec<Model>) -> Result<(), IIIError> {
        let mut slices = self.slices.lock().await; // serialized writer
        slices.insert(provider.to_string(), models);
        let value = serde_json::to_value(&*slices).unwrap_or_default();
        state_set(&self.iii, CATALOG_KEY, value).await
    }
}
