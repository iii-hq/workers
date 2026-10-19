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
pub mod browse;
pub mod catalog;
pub mod column_stats;
pub mod commit_transaction;
pub mod connections;
pub mod diagram;
pub mod distinct_values;
pub mod execute;
pub mod execute_batch;
pub mod explain;
pub mod filter;
pub mod format_sql;
pub mod health;
pub mod insert_row;
pub mod list_databases;
pub mod prepare;
pub mod query;
pub mod rollback_transaction;
pub mod run_statement;
pub mod saved;
pub mod schema;
pub mod table_view;
pub mod test_connection;
pub mod transaction;
pub mod transaction_execute;
pub mod transaction_query;
mod tx_sql_guard;

pub(crate) use query::rows_to_objects as query_rows_to_objects;

/// Reject transaction-control SQL on the pooled per-call surfaces. Each call
/// runs on whichever connection the pool hands out, so BEGIN/COMMIT can never
/// pair up — worse, a BEGIN that "succeeds" returns its connection to the
/// pool still inside an open transaction, and every later caller unlucky
/// enough to draw it fails with `cannot start a transaction within a
/// transaction` (rctest5 postmortem: three writer agents starved on one
/// poisoned connection). Point the caller at the real transactional surfaces.
pub(crate) fn reject_tx_control_sql(sql: &str) -> Result<(), DbError> {
    if tx_sql_guard::is_transaction_control_sql(sql) {
        return Err(DbError::InvalidParam {
            index: 0,
            reason: "transaction-control SQL (BEGIN/COMMIT/ROLLBACK/SAVEPOINT) is not valid \
                     here: each call runs on its own pooled connection, so the statements \
                     cannot pair up and a leaked BEGIN poisons the pool for every later \
                     caller. Use database::beginTransaction + database::transactionExecute + \
                     database::commitTransaction, or database::executeBatch for an atomic \
                     batch."
                .into(),
        });
    }
    Ok(())
}

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
    /// `database::row-changed` bindings and their fan-out. Absent when the
    /// worker runs without an engine connection (tests).
    pub row_changes: Option<Arc<crate::triggers::RowChangeBus>>,
    /// Secret cache, per-connection build errors and what each pool was
    /// built from (see `configuration::apply_config`).
    pub runtime: Arc<crate::runtime::Runtime>,
}

impl AppState {
    /// Whether `db` announces its writes via SQL classification. A
    /// `capture: native` database hears its own commits through NOTIFY like
    /// every other client — classifying here too would fire everything twice.
    async fn classifies_own_writes(&self, db: &str) -> bool {
        self.config
            .read()
            .await
            .databases
            .get(db)
            .is_none_or(|d| d.capture.is_statements())
    }

    /// Engine client, when the worker is connected to one. `None` in tests,
    /// where sibling-worker calls are not available and history is skipped.
    pub fn client(&self) -> Option<&Arc<iii_sdk::IIIClient>> {
        self.row_changes.as_ref().map(|bus| bus.client())
    }

    /// Announce a committed change, if anything is listening. Every mutating
    /// handler ends with this; the bus decides whether the statement changed
    /// rows and who cares.
    pub async fn emit_row_change(
        &self,
        db: &str,
        sql: &str,
        affected_rows: u64,
        returning: Option<&[serde_json::Map<String, serde_json::Value>]>,
    ) {
        if let Some(bus) = &self.row_changes {
            if self.classifies_own_writes(db).await {
                bus.emit(db, sql, affected_rows, returning).await;
            }
        }
    }

    /// Buffer a change made inside an interactive transaction until its commit.
    pub async fn stage_row_change(
        &self,
        transaction_id: &str,
        db: &str,
        sql: &str,
        affected_rows: u64,
        returning: Option<&[serde_json::Map<String, serde_json::Value>]>,
    ) {
        if let Some(bus) = &self.row_changes {
            if self.classifies_own_writes(db).await {
                bus.stage(transaction_id, db, sql, affected_rows, returning);
            }
        }
    }

    pub async fn commit_row_changes(&self, transaction_id: &str) {
        if let Some(bus) = &self.row_changes {
            bus.commit(transaction_id).await;
        }
    }

    pub fn drop_row_changes(&self, transaction_id: &str) {
        if let Some(bus) = &self.row_changes {
            bus.rollback(transaction_id);
        }
    }
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
        // Configured connections, failed ones included: a connection whose
        // pool could not be built is still the caller's target, and its
        // call should say why rather than pick another.
        let mut names: Vec<String> = self.pools.read().await.keys().cloned().collect();
        for name in self.runtime.errors().into_keys() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        if names.len() == 1 {
            return Ok(names.remove(0));
        }
        if names.iter().any(|n| n == crate::config::DEFAULT_DB_NAME) {
            return Ok(crate::config::DEFAULT_DB_NAME.to_string());
        }
        names.sort();
        Err(DbError::MissingDb { available: names })
    }

    pub async fn pool(&self, db: &str) -> Result<Pool, DbError> {
        let pools = self.pools.read().await;
        if let Some(pool) = pools.get(db) {
            return Ok(pool.clone());
        }
        // Configured but unbuildable: say why (the message names the field
        // or reference, never a value).
        if let Some(e) = self.runtime.error(db) {
            return Err(DbError::ConfigError {
                message: format!(
                    "connection `{db}` is unavailable ({}): {}",
                    serde_json::to_value(e.stage)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    e.message
                ),
            });
        }
        // Enumerate the registered handles so a caller that guessed wrong
        // learns the right name from one failure instead of trial-and-error.
        let mut available: Vec<String> = pools.keys().cloned().collect();
        for name in self.runtime.errors().into_keys() {
            if !available.contains(&name) {
                available.push(name);
            }
        }
        available.sort();
        Err(DbError::UnknownDb {
            db: db.to_string(),
            available,
        })
    }
}
