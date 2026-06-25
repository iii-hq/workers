//! Durable model catalog: one slice per provider, replaced wholesale on
//! reconcile. Same serialized-writer pattern as the registry — persisted via
//! the engine's `state::get`/`state::set` iii functions (src/state.rs).
//!
//! Engine-backed coverage: tests/integration.rs (reconcile, restart restore).
use std::collections::HashMap;

use crate::state::{state_get, state_set};
use crate::types::errors::is_function_not_found;
use crate::types::model::Model;
use iii_sdk::{errors::Error, IIIClient};
use tokio::sync::Mutex;

const CATALOG_KEY: &str = "catalog";

pub struct CatalogStore {
    iii: IIIClient,
    slices: Mutex<HashMap<String, Vec<Model>>>,
}

impl CatalogStore {
    pub fn new(iii: IIIClient) -> Self {
        Self {
            iii,
            slices: Mutex::new(HashMap::new()),
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

    pub async fn set_slice(&self, provider: &str, models: Vec<Model>) -> Result<(), Error> {
        let mut slices = self.slices.lock().await; // serialized writer
        slices.insert(provider.to_string(), models);
        let value = serde_json::to_value(&*slices).unwrap_or_default();
        state_set(&self.iii, CATALOG_KEY, value).await
    }
}
