//! MySQL catalog reads.
//!
//! MySQL has no namespace above the table within a connection — its "schema"
//! *is* the database the pool is connected to — so every reader scopes to
//! `DATABASE()` and reports `schema: None`. Callers get the same shape as the
//! other drivers without a redundant level.
//!
//! `information_schema.KEY_COLUMN_USAGE` already emits one row per key column
//! with `ORDINAL_POSITION`, and `STATISTICS` one row per indexed column with
//! `SEQ_IN_INDEX`, so composite keys and multi-column indexes pair correctly
//! without any array handling or `GROUP_CONCAT`.

use super::{
    bool_at, fold_index_rows, i64_at, run, str_at, ColumnDesc, ForeignKeyRef, IndexDesc,
    TableFilter, TableKey, TableKind, TableRef,
};
use crate::handlers::AppState;
use serde_json::Value;
use std::collections::HashMap;

fn filter_clause(filter: Option<&TableFilter>, column: &str) -> (String, Vec<Value>) {
    match filter {
        Some(f) => (
            format!(" AND {column} = ?"),
            vec![Value::String(f.table.clone())],
        ),
        None => (String::new(), Vec::new()),
    }
}

fn key_of(row: &serde_json::Map<String, Value>) -> Option<TableKey> {
    Some((None, str_at(row, "table_name")?))
}

pub async fn list_tables(
    state: &AppState,
    db: &str,
    timeout_ms: u64,
) -> Result<Vec<TableRef>, String> {
    let sql = "SELECT TABLE_NAME AS table_name, TABLE_TYPE AS kind \
               FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() \
               ORDER BY TABLE_TYPE, TABLE_NAME";
    let rows = run(state, db, sql, vec![], timeout_ms).await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(TableRef {
                name: str_at(r, "table_name")?,
                schema: None,
                kind: match str_at(r, "kind").as_deref() {
                    Some("VIEW") => TableKind::View,
                    _ => TableKind::Table,
                },
            })
        })
        .collect())
}

pub async fn columns(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
) -> Result<HashMap<TableKey, Vec<ColumnDesc>>, String> {
    let (clause, params) = filter_clause(filter, "TABLE_NAME");
    let sql = format!(
        "SELECT TABLE_NAME AS table_name, COLUMN_NAME AS column_name, \
                COLUMN_TYPE AS column_type, IS_NULLABLE AS is_nullable, \
                COLUMN_DEFAULT AS default_value, ORDINAL_POSITION AS position, \
                COLUMN_KEY AS column_key \
         FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = DATABASE(){clause} \
         ORDER BY TABLE_NAME, ORDINAL_POSITION"
    );
    let rows = run(state, db, sql, params, timeout_ms).await?;

    let mut out: HashMap<TableKey, Vec<ColumnDesc>> = HashMap::new();
    for r in &rows {
        let (Some(key), Some(name)) = (key_of(r), str_at(r, "column_name")) else {
            continue;
        };
        out.entry(key).or_default().push(ColumnDesc {
            name,
            ty: str_at(r, "column_type").unwrap_or_default(),
            // IS_NULLABLE is the string 'YES'/'NO'.
            nullable: bool_at(r, "is_nullable"),
            default_value: str_at(r, "default_value"),
            // COLUMN_KEY is 'PRI' for a primary-key member. Cheaper and more
            // reliable than a second trip to KEY_COLUMN_USAGE.
            primary_key: str_at(r, "column_key").as_deref() == Some("PRI"),
            position: i64_at(r, "position").unwrap_or(0) as i32,
            foreign_key: None,
        });
    }

    merge_foreign_keys(state, db, filter, timeout_ms, &mut out).await?;
    Ok(out)
}

async fn merge_foreign_keys(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
    columns: &mut HashMap<TableKey, Vec<ColumnDesc>>,
) -> Result<(), String> {
    let (clause, params) = filter_clause(filter, "TABLE_NAME");
    let sql = format!(
        "SELECT TABLE_NAME AS table_name, COLUMN_NAME AS src_column, \
                REFERENCED_TABLE_NAME AS ref_table, REFERENCED_COLUMN_NAME AS ref_column \
         FROM information_schema.KEY_COLUMN_USAGE \
         WHERE TABLE_SCHEMA = DATABASE() AND REFERENCED_TABLE_NAME IS NOT NULL{clause} \
         ORDER BY TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION"
    );
    for r in run(state, db, sql, params, timeout_ms).await? {
        let (Some(key), Some(src), Some(ref_table), Some(ref_column)) = (
            key_of(&r),
            str_at(&r, "src_column"),
            str_at(&r, "ref_table"),
            str_at(&r, "ref_column"),
        ) else {
            continue;
        };
        if let Some(cols) = columns.get_mut(&key) {
            if let Some(c) = cols.iter_mut().find(|c| c.name == src) {
                c.foreign_key = Some(ForeignKeyRef {
                    schema: None,
                    table: ref_table,
                    column: ref_column,
                });
            }
        }
    }
    Ok(())
}

pub async fn indexes(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
) -> Result<HashMap<TableKey, Vec<IndexDesc>>, String> {
    let (clause, params) = filter_clause(filter, "TABLE_NAME");
    // NON_UNIQUE is inverted, so flip it into the `is_unique` the fold reads.
    // MySQL names the primary-key index 'PRIMARY' and offers no other flag.
    let sql = format!(
        "SELECT TABLE_NAME AS table_name, INDEX_NAME AS index_name, \
                NON_UNIQUE = 0 AS is_unique, INDEX_NAME = 'PRIMARY' AS is_primary, \
                COLUMN_NAME AS column_name \
         FROM information_schema.STATISTICS \
         WHERE TABLE_SCHEMA = DATABASE(){clause} \
         ORDER BY TABLE_NAME, INDEX_NAME, SEQ_IN_INDEX"
    );
    let rows = run(state, db, sql, params, timeout_ms).await?;
    Ok(fold_index_rows(&rows, key_of))
}

/// `TABLE_ROWS` is an InnoDB estimate sampled from the index, and is NULL for
/// views. Treat NULL as absent rather than zero — "unknown" and "empty" are
/// different answers.
pub async fn row_estimates(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
) -> Result<HashMap<TableKey, i64>, String> {
    let (clause, params) = filter_clause(filter, "TABLE_NAME");
    let sql = format!(
        "SELECT TABLE_NAME AS table_name, TABLE_ROWS AS estimate \
         FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_ROWS IS NOT NULL{clause}"
    );
    let mut out = HashMap::new();
    for r in run(state, db, sql, params, timeout_ms).await? {
        if let (Some(key), Some(est)) = (key_of(&r), i64_at(&r, "estimate")) {
            out.insert(key, est);
        }
    }
    Ok(out)
}
