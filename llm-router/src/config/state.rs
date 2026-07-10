//! Hot-swappable, in-memory snapshot of the complete `llm-router`
//! configuration entry.
//!
//! Request handlers clone the inner `Arc` under a short read lock, so they
//! never hold the lock while doing async work. Configuration changes replace
//! the whole snapshot after re-fetching the authoritative stored value.
use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::settings::{parse_settings, RouterSettings};

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSnapshot {
    value: Value,
    settings: RouterSettings,
}

impl ConfigSnapshot {
    pub fn from_value(value: Value) -> Self {
        let settings = parse_settings(&value);
        Self { value, settings }
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn settings(&self) -> &RouterSettings {
        &self.settings
    }

    pub fn provider_slice(&self, provider: &str) -> Option<&Value> {
        self.value
            .get("providers")
            .and_then(Value::as_object)
            .and_then(|providers| providers.get(provider))
            .filter(|slice| slice.is_object())
    }
}

impl Default for ConfigSnapshot {
    fn default() -> Self {
        Self::from_value(Value::Null)
    }
}

/// Live configuration shared by every router handler.
pub type ConfigCell = Arc<RwLock<Arc<ConfigSnapshot>>>;

pub fn new_config_cell(value: Value) -> ConfigCell {
    Arc::new(RwLock::new(Arc::new(ConfigSnapshot::from_value(value))))
}

pub fn snapshot(cell: &ConfigCell) -> Arc<ConfigSnapshot> {
    cell.read().unwrap().clone()
}

pub fn apply_config(cell: &ConfigCell, value: Value) {
    *cell.write().unwrap() = Arc::new(ConfigSnapshot::from_value(value));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn swaps_the_full_entry_and_derived_settings_together() {
        let cell = new_config_cell(json!({
            "default_provider": "before",
            "providers": { "before": { "api_key": "old" } }
        }));
        let previous = snapshot(&cell);

        apply_config(
            &cell,
            json!({
                "default_provider": "after",
                "providers": { "after": { "api_key": "new" } }
            }),
        );

        let current = snapshot(&cell);
        assert_eq!(
            previous.settings().default_provider.as_deref(),
            Some("before")
        );
        assert_eq!(
            current.settings().default_provider.as_deref(),
            Some("after")
        );
        assert_eq!(
            current.provider_slice("after").unwrap()["api_key"],
            json!("new")
        );
        assert!(current.provider_slice("before").is_none());
    }
}
