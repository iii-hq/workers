//! TriggerHandler implementations for `storage::object-created` and
//! `storage::object-deleted`. These are thin: they parse the bucket out
//! of the per-instance config and add/remove a (bucket, kind) → function_id
//! entry in the shared registry. Upstream pollers are reconciled whenever the
//! storage configuration changes; see `configuration.rs`.

use crate::triggers::normalize::EventKind;
use crate::triggers::object_created::TriggerConfig as CreatedConfig;
use crate::triggers::object_deleted::TriggerConfig as DeletedConfig;
use crate::triggers::registry::TriggerRegistry;
use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type WiredBuckets = Arc<RwLock<HashSet<String>>>;

pub struct ObjectCreatedHandler {
    pub registry: Arc<TriggerRegistry>,
    /// Live set of buckets whose notifications source is currently wired.
    /// Registering a trigger for a bucket missing from this set fails fast
    /// instead of silently never firing.
    pub wired_buckets: WiredBuckets,
    pub reconfigure_gate: Arc<RwLock<()>>,
}

#[async_trait]
impl TriggerHandler for ObjectCreatedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let _gate = self.reconfigure_gate.read().await;
        let cfg: CreatedConfig = serde_json::from_value(config.config.clone()).map_err(|e| {
            // Build the envelope through serde_json so internal quotes,
            // newlines, and other JSON-special chars in `e` are escaped
            // rather than producing malformed JSON.
            Error::Handler(
                serde_json::json!({
                    "code": "CONFIG_ERROR",
                    "message": format!("object-created config: {e}"),
                })
                .to_string(),
            )
        })?;
        if !self.wired_buckets.read().await.contains(&cfg.bucket) {
            return Err(Error::Handler(
                serde_json::json!({
                    "code": "CONFIG_ERROR",
                    "message": format!(
                        "bucket `{}` has no notifications source configured; add `notifications:` under the bucket in worker config",
                        cfg.bucket
                    ),
                })
                .to_string(),
            ));
        }
        self.registry.register(
            cfg.bucket,
            EventKind::Created,
            config.id.clone(),
            config.function_id.clone(),
            cfg.handler_timeout_ms,
        );
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            "object-created trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.registry.unregister(&config.id);
        tracing::info!(instance = %config.id, "object-created trigger unregistered");
        Ok(())
    }
}

pub struct ObjectDeletedHandler {
    pub registry: Arc<TriggerRegistry>,
    pub wired_buckets: WiredBuckets,
    pub reconfigure_gate: Arc<RwLock<()>>,
}

#[async_trait]
impl TriggerHandler for ObjectDeletedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let _gate = self.reconfigure_gate.read().await;
        let cfg: DeletedConfig = serde_json::from_value(config.config.clone()).map_err(|e| {
            Error::Handler(
                serde_json::json!({
                    "code": "CONFIG_ERROR",
                    "message": format!("object-deleted config: {e}"),
                })
                .to_string(),
            )
        })?;
        if !self.wired_buckets.read().await.contains(&cfg.bucket) {
            return Err(Error::Handler(
                serde_json::json!({
                    "code": "CONFIG_ERROR",
                    "message": format!(
                        "bucket `{}` has no notifications source configured; add `notifications:` under the bucket in worker config",
                        cfg.bucket
                    ),
                })
                .to_string(),
            ));
        }
        self.registry.register(
            cfg.bucket,
            EventKind::Deleted,
            config.id.clone(),
            config.function_id.clone(),
            cfg.handler_timeout_ms,
        );
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            "object-deleted trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.registry.unregister(&config.id);
        tracing::info!(instance = %config.id, "object-deleted trigger unregistered");
        Ok(())
    }
}
