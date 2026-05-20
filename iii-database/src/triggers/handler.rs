//! TriggerHandler implementations for `iii-database::row-change`. Wired into
//! the worker via `iii.register_trigger_type` from main.rs.

use async_trait::async_trait;
use iii_sdk::{IIIError, TriggerConfig, TriggerHandler};

fn iii_err<T: serde::Serialize>(err: T) -> IIIError {
    IIIError::Handler(serde_json::to_string(&err).unwrap_or_else(|_| "{}".into()))
}

/// `iii-database::row-change` trigger handler. v1.0 stubs the streaming decoder
/// pending an upstream tokio-postgres replication API release. `register_trigger`
/// returns Unsupported so callers see a clear error instead of silently never
/// receiving events.
pub struct RowChangeTrigger;

#[async_trait]
impl TriggerHandler for RowChangeTrigger {
    async fn register_trigger(&self, _config: TriggerConfig) -> Result<(), IIIError> {
        Err(iii_err(crate::error::DbError::Unsupported {
            op: "row-change".into(),
            driver: "postgres (pending tokio-postgres replication API release)".into(),
        }))
    }
    async fn unregister_trigger(&self, _config: TriggerConfig) -> Result<(), IIIError> {
        Ok(())
    }
}
