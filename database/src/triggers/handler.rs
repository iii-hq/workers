//! TriggerHandler implementations for `database::row-change`. Wired into
//! the worker via `iii.register_trigger_type` from main.rs.

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};

fn iii_err<T: serde::Serialize>(err: T) -> Error {
    Error::Handler(serde_json::to_string(&err).unwrap_or_else(|_| "{}".into()))
}

/// `database::row-change` trigger handler. v1.0 stubs the streaming decoder
/// pending an upstream tokio-postgres replication API release. `register_trigger`
/// returns Unsupported so callers see a clear error instead of silently never
/// receiving events.
pub struct RowChangeTrigger;

#[async_trait]
impl TriggerHandler for RowChangeTrigger {
    async fn register_trigger(&self, _config: TriggerConfig) -> Result<(), Error> {
        Err(iii_err(crate::error::DbError::Unsupported {
            op: "row-change".into(),
            driver: "postgres (pending tokio-postgres replication API release)".into(),
        }))
    }
    async fn unregister_trigger(&self, _config: TriggerConfig) -> Result<(), Error> {
        Ok(())
    }
}
