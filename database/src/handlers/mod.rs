//! RPC handlers for `database::*` functions. Each handler accepts a JSON
//! payload from the SDK, validates it, dispatches to the configured pool,
//! and serializes the result.

use crate::error::DbError;
use crate::handle::HandleRegistry;
use crate::pool::Pool;
use crate::transaction::TxRegistry;
use iii_sdk::Logger;
use std::collections::HashMap;
use std::sync::Arc;

pub mod begin_transaction;
pub mod commit_transaction;
pub mod execute;
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
    pub pools: Arc<HashMap<String, Pool>>,
    pub handles: Arc<HandleRegistry>,
    pub transactions: TxRegistry,
    pub log: Logger,
}

impl AppState {
    pub fn pool(&self, db: &str) -> Result<&Pool, DbError> {
        self.pools
            .get(db)
            .ok_or_else(|| DbError::UnknownDb { db: db.to_string() })
    }
}
