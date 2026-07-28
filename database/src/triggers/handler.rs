//! The `database::row-changed` TriggerHandler.
//!
//! Thin by design: the engine hands a registration here, this validates the
//! config and files it in the [`RowChangeBus`]; the mutating handlers do the
//! emitting. Registration fails loudly for a database that is not configured —
//! a binding on a typo'd handle would otherwise sit there listening to nothing.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};

use super::bus::{RowChangeBus, RowChangedConfig};
use crate::config::WorkerConfig;

pub struct RowChangedHandler {
    pub bus: Arc<RowChangeBus>,
    /// Live configuration, swapped together with the pools on hot reload.
    pub config: Arc<tokio::sync::RwLock<WorkerConfig>>,
}

fn config_error(message: String) -> Error {
    Error::Handler(serde_json::json!({ "code": "CONFIG_ERROR", "message": message }).to_string())
}

#[async_trait]
impl TriggerHandler for RowChangedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let cfg: RowChangedConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| config_error(format!("row-changed config: {e}")))?;

        let live = self.config.read().await;
        if !live.databases.contains_key(&cfg.db) {
            let mut known = live.databases.keys().cloned().collect::<Vec<_>>();
            known.sort();
            return Err(config_error(format!(
                "unknown db `{}`; available: [{}]",
                cfg.db,
                known.join(", ")
            )));
        }
        drop(live);

        let table = cfg.table.clone();
        self.bus.register(
            config.id.clone(),
            config.function_id.clone(),
            config.metadata.clone(),
            cfg,
        );
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            table = ?table,
            "row-changed trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.bus.unregister(&config.id);
        tracing::info!(instance = %config.id, "row-changed trigger unregistered");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(id: &str, db: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: "app::on-change".into(),
            config: serde_json::json!({ "db": db }),
            metadata: None,
        }
    }

    #[tokio::test]
    async fn registration_uses_the_live_database_config() {
        let config = Arc::new(tokio::sync::RwLock::new(WorkerConfig::default()));
        let bus = Arc::new(RowChangeBus::new(
            Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")),
            100,
        ));
        let handler = RowChangedHandler {
            bus,
            config: config.clone(),
        };

        handler
            .register_trigger(trigger("initial", "primary"))
            .await
            .unwrap();

        let mut live = config.write().await;
        let db = live.databases.remove("primary").unwrap();
        live.databases.insert("analytics".into(), db);
        drop(live);

        assert!(handler
            .register_trigger(trigger("removed", "primary"))
            .await
            .is_err());
        handler
            .register_trigger(trigger("added", "analytics"))
            .await
            .unwrap();
    }
}
