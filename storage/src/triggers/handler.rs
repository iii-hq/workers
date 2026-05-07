//! TriggerHandler implementations for `storage::object-created` and
//! `storage::object-deleted`. These are thin: they parse the bucket out
//! of the per-instance config and add/remove a (bucket, kind) → function_id
//! entry in the shared registry. The actual upstream polling runs once per
//! provider source at worker startup; see `main.rs`.

use crate::triggers::normalize::EventKind;
use crate::triggers::object_created::TriggerConfig as CreatedConfig;
use crate::triggers::object_deleted::TriggerConfig as DeletedConfig;
use crate::triggers::registry::TriggerRegistry;
use async_trait::async_trait;
use iii_sdk::{IIIError, TriggerConfig, TriggerHandler};
use std::sync::Arc;

pub struct ObjectCreatedHandler {
    pub registry: Arc<TriggerRegistry>,
    /// Set of buckets whose notifications source is wired up at startup.
    /// Registering a trigger for a bucket missing from this set fails fast
    /// instead of silently never firing.
    pub wired_buckets: Arc<std::collections::HashSet<String>>,
}

#[async_trait]
impl TriggerHandler for ObjectCreatedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let cfg: CreatedConfig = serde_json::from_value(config.config.clone()).map_err(|e| {
            // Build the envelope through serde_json so internal quotes,
            // newlines, and other JSON-special chars in `e` are escaped
            // rather than producing malformed JSON.
            IIIError::Handler(
                serde_json::json!({
                    "code": "CONFIG_ERROR",
                    "message": format!("object-created config: {e}"),
                })
                .to_string(),
            )
        })?;
        if !self.wired_buckets.contains(&cfg.bucket) {
            return Err(IIIError::Handler(
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

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        self.registry.unregister(&config.id);
        tracing::info!(instance = %config.id, "object-created trigger unregistered");
        Ok(())
    }
}

pub struct ObjectDeletedHandler {
    pub registry: Arc<TriggerRegistry>,
    pub wired_buckets: Arc<std::collections::HashSet<String>>,
}

#[async_trait]
impl TriggerHandler for ObjectDeletedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let cfg: DeletedConfig = serde_json::from_value(config.config.clone()).map_err(|e| {
            IIIError::Handler(
                serde_json::json!({
                    "code": "CONFIG_ERROR",
                    "message": format!("object-deleted config: {e}"),
                })
                .to_string(),
            )
        })?;
        if !self.wired_buckets.contains(&cfg.bucket) {
            return Err(IIIError::Handler(
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

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        self.registry.unregister(&config.id);
        tracing::info!(instance = %config.id, "object-deleted trigger unregistered");
        Ok(())
    }
}
