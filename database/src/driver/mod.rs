//! Shared driver-facing types. Each driver (postgres / mysql / sqlite)
//! exposes async functions returning these types so the dispatch layer in
//! `pool::Pool` is uniform.

use crate::value::{JsonParam, RowValue};
use serde::Serialize;

pub mod mysql;
pub mod postgres;
pub mod sqlite;

#[derive(Debug, Clone, Serialize)]
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
}

#[derive(Debug, Default)]
pub struct ExecuteResult {
    pub affected_rows: u64,
    pub last_insert_id: Option<String>,
    pub returned_rows: Vec<Row>,
    pub returned_columns: Vec<ColumnMeta>,
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
}
