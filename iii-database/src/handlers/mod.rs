//! RPC handlers for `database::*` functions. Each handler accepts a JSON
//! payload from the SDK, validates it, dispatches to the configured pool,
//! and serializes the result.

use crate::error::DbError;
use crate::handle::HandleRegistry;
use crate::pool::Pool;
use std::collections::HashMap;
use std::sync::Arc;

pub mod execute;
pub mod prepare;
pub mod query;
pub mod run_statement;
pub mod transaction;

pub(crate) use query::rows_to_objects as query_rows_to_objects;

#[derive(Clone)]
pub struct AppState {
    pub pools: Arc<HashMap<String, Pool>>,
    pub handles: Arc<HandleRegistry>,
}

impl AppState {
    pub fn pool(&self, db: &str) -> Result<&Pool, DbError> {
        self.pools
            .get(db)
            .ok_or_else(|| DbError::UnknownDb { db: db.to_string() })
    }
}
