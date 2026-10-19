//! Shared driver-facing types. Each driver (postgres / mysql / sqlite)
//! exposes async functions returning these types so the dispatch layer in
//! `pool::Pool` is uniform.

use crate::value::{JsonParam, RowValue};
use schemars::JsonSchema;
use serde::Serialize;

pub mod mysql;
pub mod postgres;
pub mod sqlite;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ColumnMeta {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone)]
pub struct Row(pub Vec<RowValue>);

#[derive(Debug)]
pub struct QueryResult {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
    /// More rows existed past the `max_rows` cap (always false without one).
    pub truncated: bool,
    /// Time spent running the statement and reading its rows, pool
    /// acquisition excluded.
    pub elapsed_ms: u64,
}

/// How `query` runs a read.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadOptions {
    pub timeout_ms: u64,
    /// Stop reading after this many rows (one more is read to learn whether
    /// the result was cut). `None` reads everything.
    pub max_rows: Option<u64>,
    /// SQLite: also run the statement with `PRAGMA query_only` on, so even a
    /// function with side effects cannot write. (Postgres and MySQL already
    /// run every read in a read-only transaction.)
    pub query_only: bool,
}

#[derive(Debug, Default)]
pub struct ExecuteResult {
    pub affected_rows: u64,
    pub last_insert_id: Option<String>,
    pub returned_rows: Vec<Row>,
    pub returned_columns: Vec<ColumnMeta>,
    /// Time spent running the statement, pool acquisition excluded. Set by
    /// `execute`; zero elsewhere.
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum Isolation {
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

#[derive(Debug)]
pub struct TxStatement {
    pub sql: String,
    pub params: Vec<JsonParam>,
}

#[derive(Debug)]
pub struct TxStepResult {
    pub affected_rows: u64,
    pub rows: Vec<Row>,
    pub columns: Vec<ColumnMeta>,
}
