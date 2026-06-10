//! Durable model catalog: one slice per provider, replaced wholesale on
//! reconcile. Same serialized-writer pattern as the registry — persisted via
//! the engine's `state::get`/`state::set` iii functions.
use std::collections::HashMap;
use std::sync::Arc;

use crate::types::model::Model;
use serde_json::json;
use tokio::sync::Mutex;

use crate::bus::{Bus, BusError};
use crate::registry::store::STATE_SCOPE;

const CATALOG_KEY: &str = "catalog";

pub struct CatalogStore {
    bus: Arc<dyn Bus>,
    slices: Mutex<HashMap<String, Vec<Model>>>,
}

impl CatalogStore {
    pub fn new(bus: Arc<dyn Bus>) -> Self {
        Self {
            bus,
            slices: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Result<(), BusError> {
        let stored = self
            .bus
            .trigger(
                "state::get",
                json!({ "scope": STATE_SCOPE, "key": CATALOG_KEY }),
                None,
            )
            .await?;
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

    pub async fn set_slice(&self, provider: &str, models: Vec<Model>) -> Result<(), BusError> {
        let mut slices = self.slices.lock().await; // serialized writer
        slices.insert(provider.to_string(), models);
        let value = serde_json::to_value(&*slices).unwrap_or_default();
        self.bus
            .trigger(
                "state::set",
                json!({ "scope": STATE_SCOPE, "key": CATALOG_KEY, "value": value }),
                None,
            )
            .await?;
        Ok(())
    }
}
