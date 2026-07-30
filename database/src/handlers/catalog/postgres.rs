//! PostgreSQL catalog reads.
//!
//! Reads `pg_catalog` rather than `information_schema`. Two reasons: it is
//! markedly faster, and `information_schema.constraint_column_usage` pairs
//! composite foreign keys by cross product, which mis-associates columns on a
//! multi-column key. `pg_constraint` carries `conkey`/`confkey` as ordered
//! vectors, so unnesting both `WITH ORDINALITY` and matching on the ordinal
//! pairs them correctly.
//!
//! Nothing here returns an array-typed column: `RowValue` has no array
//! variant, so `array_agg`, a bare `conkey`, or `indkey` would fail to decode.
//! Arrays are unnested into scalar rows and regrouped in Rust.

use super::{
    bool_at, fold_index_rows, i64_at, run, str_at, ColumnDesc, ForeignKeyRef, IndexDesc,
    TableFilter, TableKey, TableKind, TableRef,
};
use crate::handlers::AppState;
use serde_json::Value;
use std::collections::HashMap;

const SKIP_SCHEMAS: &str = "n.nspname NOT IN ('pg_catalog', 'information_schema')";

/// Build the optional `AND schema = $n AND table = $m` tail. Postgres uses
/// positional placeholders, so the caller's first free index is passed in.
fn filter_clause(filter: Option<&TableFilter>, first_param: usize) -> (String, Vec<Value>) {
    let Some(f) = filter else {
        return (String::new(), Vec::new());
    };
    let mut params = Vec::new();
    let mut clause = String::new();
    let mut n = first_param;
    if let Some(schema) = &f.schema {
        clause.push_str(&format!(" AND n.nspname = ${n}"));
        params.push(Value::String(schema.clone()));
        n += 1;
    }
    clause.push_str(&format!(" AND c.relname = ${n}"));
    params.push(Value::String(f.table.clone()));
    (clause, params)
}

fn key_of(row: &serde_json::Map<String, Value>) -> Option<TableKey> {
    Some((str_at(row, "table_schema"), str_at(row, "table_name")?))
}

pub async fn list_tables(
    state: &AppState,
    db: &str,
    timeout_ms: u64,
) -> Result<Vec<TableRef>, String> {
    let sql = format!(
        "SELECT n.nspname AS table_schema, c.relname AS table_name, c.relkind AS kind \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE c.relkind IN ('r', 'p', 'v', 'm') AND {SKIP_SCHEMAS} \
         ORDER BY n.nspname, c.relname"
    );
    let rows = run(state, db, sql, vec![], timeout_ms).await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(TableRef {
                name: str_at(r, "table_name")?,
                schema: str_at(r, "table_schema"),
                // r = ordinary table, p = partitioned, v = view, m = materialized view
                kind: match str_at(r, "kind").as_deref() {
                    Some("v") | Some("m") => TableKind::View,
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
    let (clause, params) = filter_clause(filter, 1);
    let sql = format!(
        "SELECT n.nspname AS table_schema, c.relname AS table_name, \
                a.attname AS column_name, \
                format_type(a.atttypid, a.atttypmod) AS column_type, \
                NOT a.attnotnull AS is_nullable, \
                pg_get_expr(d.adbin, d.adrelid) AS default_value, \
                a.attnum AS position \
         FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum \
         WHERE a.attnum > 0 AND NOT a.attisdropped \
           AND c.relkind IN ('r', 'p', 'v', 'm') AND {SKIP_SCHEMAS}{clause} \
         ORDER BY n.nspname, c.relname, a.attnum"
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
            nullable: bool_at(r, "is_nullable"),
            default_value: str_at(r, "default_value"),
            primary_key: false,
            position: i64_at(r, "position").unwrap_or(0) as i32,
            foreign_key: None,
        });
    }

    mark_primary_keys(state, db, filter, timeout_ms, &mut out).await?;
    merge_foreign_keys(state, db, filter, timeout_ms, &mut out).await?;
    Ok(out)
}

async fn mark_primary_keys(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
    columns: &mut HashMap<TableKey, Vec<ColumnDesc>>,
) -> Result<(), String> {
    let (clause, params) = filter_clause(filter, 1);
    let sql = format!(
        "SELECT n.nspname AS table_schema, c.relname AS table_name, a.attname AS column_name \
         FROM pg_constraint con \
         JOIN pg_class c ON c.oid = con.conrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         CROSS JOIN LATERAL unnest(con.conkey) AS k(attnum) \
         JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
         WHERE con.contype = 'p' AND {SKIP_SCHEMAS}{clause}"
    );
    for r in run(state, db, sql, params, timeout_ms).await? {
        let (Some(key), Some(col)) = (key_of(&r), str_at(&r, "column_name")) else {
            continue;
        };
        if let Some(cols) = columns.get_mut(&key) {
            if let Some(c) = cols.iter_mut().find(|c| c.name == col) {
                c.primary_key = true;
            }
        }
    }
    Ok(())
}

async fn merge_foreign_keys(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
    columns: &mut HashMap<TableKey, Vec<ColumnDesc>>,
) -> Result<(), String> {
    let (clause, params) = filter_clause(filter, 1);
    // `k.ord = fk.ord` is what makes a composite key pair correctly — without
    // it the two unnests cross-product and column 1 can be reported as
    // referencing the parent's column 2.
    let sql = format!(
        "SELECT n.nspname AS table_schema, c.relname AS table_name, \
                a.attname AS src_column, \
                fn.nspname AS ref_schema, fc.relname AS ref_table, fa.attname AS ref_column \
         FROM pg_constraint con \
         JOIN pg_class c ON c.oid = con.conrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_class fc ON fc.oid = con.confrelid \
         JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
         CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
         CROSS JOIN LATERAL unnest(con.confkey) WITH ORDINALITY AS fk(attnum, ord) \
         JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
         JOIN pg_attribute fa ON fa.attrelid = con.confrelid AND fa.attnum = fk.attnum \
         WHERE con.contype = 'f' AND k.ord = fk.ord AND {SKIP_SCHEMAS}{clause}"
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
                    schema: str_at(&r, "ref_schema"),
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
    let (clause, params) = filter_clause(filter, 1);
    // `indkey` is an int2vector; unnest it rather than returning it, and keep
    // the LEFT JOIN so an expression index still yields its row with no
    // column name attached.
    let sql = format!(
        "SELECT n.nspname AS table_schema, c.relname AS table_name, \
                i.relname AS index_name, ix.indisunique AS is_unique, \
                ix.indisprimary AS is_primary, a.attname AS column_name \
         FROM pg_index ix \
         JOIN pg_class c ON c.oid = ix.indrelid \
         JOIN pg_class i ON i.oid = ix.indexrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord) \
         LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE {SKIP_SCHEMAS}{clause} \
         ORDER BY n.nspname, c.relname, i.relname, k.ord"
    );
    let rows = run(state, db, sql, params, timeout_ms).await?;
    Ok(fold_index_rows(&rows, key_of))
}

/// `reltuples` is the planner's estimate, maintained by ANALYZE/autovacuum.
/// It is -1 on a table that has never been analyzed, which we report as
/// absent rather than as a row count of minus one.
pub async fn row_estimates(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
) -> Result<HashMap<TableKey, i64>, String> {
    let (clause, params) = filter_clause(filter, 1);
    let sql = format!(
        "SELECT n.nspname AS table_schema, c.relname AS table_name, \
                c.reltuples::bigint AS estimate \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE c.relkind IN ('r', 'p') AND {SKIP_SCHEMAS}{clause}"
    );
    let mut out = HashMap::new();
    for r in run(state, db, sql, params, timeout_ms).await? {
        if let (Some(key), Some(est)) = (key_of(&r), i64_at(&r, "estimate")) {
            if est >= 0 {
                out.insert(key, est);
            }
        }
    }
    Ok(out)
}
