//! RPC handlers for `database::*` functions. Each handler accepts a JSON
//! payload from the SDK, validates it, dispatches to the configured pool,
//! and serializes the result.

use crate::config::WorkerConfig;
use crate::error::DbError;
use crate::handle::HandleRegistry;
use crate::pool::Pool;
use crate::transaction::TxRegistry;
use iii_helpers::observability::Logger;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub(crate) fn lenient_params<'de, D>(de: D) -> Result<Vec<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    use serde::Deserialize as _;

    match serde_json::Value::deserialize(de)? {
        serde_json::Value::Array(items) => Ok(items),
        serde_json::Value::String(s) => serde_json::from_str(&s)
            .map_err(|e| D::Error::custom(format!("params string is not a JSON array: {e}"))),
        _ => Err(D::Error::custom(
            "params must be a JSON array or a string containing one",
        )),
    }
}

pub mod begin_transaction;
pub mod commit_transaction;
pub mod execute;
pub mod execute_batch;
pub mod list_databases;
pub mod prepare;
pub mod query;
pub mod rollback_transaction;
pub mod run_statement;
pub mod transaction;
pub mod transaction_execute;
pub mod transaction_query;
mod tx_sql_guard;

pub(crate) use query::rows_to_objects as query_rows_to_objects;

#[derive(Clone)]
pub struct AppState {
    pub pools: Arc<RwLock<HashMap<String, Pool>>>,
    /// Live config snapshot the pools were built from. Swapped together with
    /// `pools` on hot-reload (see `configuration::apply_config`) so readers
    /// never observe new pools paired with stale config.
    pub config: Arc<RwLock<WorkerConfig>>,
    pub handles: Arc<HandleRegistry>,
    pub transactions: TxRegistry,
    pub log: Logger,
}

impl AppState {
    /// Resolve an optional logical db name to a concrete pool key.
    ///
    /// `Some(name)` passes through untouched (unknown names still surface
    /// `UNKNOWN_DB` from [`AppState::pool`]). `None` falls back to the sole
    /// configured pool, then to `primary` when several exist; the omission is
    /// only an error (`MISSING_DB`) when neither rule disambiguates. This
    /// exists because LLM callers routinely omit `db` on their first call —
    /// a hard serde failure there is a wasted round-trip on every session.
    pub async fn resolve_db(&self, db: Option<String>) -> Result<String, DbError> {
        if let Some(name) = db {
            return Ok(name);
        }
        let pools = self.pools.read().await;
        if pools.len() == 1 {
            return Ok(pools.keys().next().expect("len checked above").clone());
        }
        if pools.contains_key(crate::config::DEFAULT_DB_NAME) {
            return Ok(crate::config::DEFAULT_DB_NAME.to_string());
        }
        let mut available: Vec<String> = pools.keys().cloned().collect();
        available.sort();
        Err(DbError::MissingDb { available })
    }

    pub async fn pool(&self, db: &str) -> Result<Pool, DbError> {
        let pools = self.pools.read().await;
        pools.get(db).cloned().ok_or_else(|| {
            // Enumerate the registered handles so a caller that guessed wrong
            // learns the right name from one failure instead of trial-and-error.
            let mut available: Vec<String> = pools.keys().cloned().collect();
            available.sort();
            DbError::UnknownDb {
                db: db.to_string(),
                available,
            }
        })
    }
}
