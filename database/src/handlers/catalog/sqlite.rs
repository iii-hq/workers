//! SQLite catalog reads.
//!
//! Uses the table-valued PRAGMA functions (`pragma_table_info(name)`, SQLite
//! 3.16+) joined against `sqlite_master` rather than issuing a bare `PRAGMA`
//! per table. That keeps `describeSchema` to three queries instead of one per
//! table, and it is the only way to read the catalog with bound parameters —
//! a bare `PRAGMA table_info(x)` cannot bind `x`.

use super::{
    bool_at, fold_index_rows, i64_at, run, str_at, ColumnDesc, ForeignKeyRef, IndexDesc,
    TableFilter, TableKey, TableKind, TableRef,
};
use crate::handlers::AppState;
use serde_json::Value;
use std::collections::HashMap;

/// `sqlite_master` rows we never surface: the internal bookkeeping tables.
const NOT_INTERNAL: &str = "m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\'";

fn filter_clause(filter: Option<&TableFilter>) -> (String, Vec<Value>) {
    match filter {
        Some(f) => (
            " AND m.name = ?".to_string(),
            vec![Value::String(f.table.clone())],
        ),
        None => (String::new(), Vec::new()),
    }
}

pub async fn list_tables(
    state: &AppState,
    db: &str,
    timeout_ms: u64,
) -> Result<Vec<TableRef>, String> {
    let sql = format!(
        "SELECT m.name AS name, m.type AS kind FROM sqlite_master m \
         WHERE m.type IN ('table', 'view') AND {NOT_INTERNAL} ORDER BY m.type, m.name"
    );
    let rows = run(state, db, sql, vec![], timeout_ms).await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(TableRef {
                name: str_at(r, "name")?,
                schema: None,
                kind: match str_at(r, "kind").as_deref() {
                    Some("view") => TableKind::View,
                    _ => TableKind::Table,
                },
            })
        })
        .collect())
}

/// Columns for one table or every table, with primary keys and foreign keys
/// already merged in.
pub async fn columns(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
) -> Result<HashMap<TableKey, Vec<ColumnDesc>>, String> {
    let (clause, params) = filter_clause(filter);
    let sql = format!(
        "SELECT m.name AS table_name, p.cid AS cid, p.name AS column_name, \
                p.type AS column_type, p.\"notnull\" AS not_null, \
                p.dflt_value AS default_value, p.pk AS pk_ord \
         FROM sqlite_master m JOIN pragma_table_info(m.name) p \
         WHERE m.type IN ('table', 'view') AND {NOT_INTERNAL}{clause} \
         ORDER BY m.name, p.cid"
    );
    let rows = run(state, db, sql, params, timeout_ms).await?;

    let mut out: HashMap<TableKey, Vec<ColumnDesc>> = HashMap::new();
    for r in &rows {
        let Some(table) = str_at(r, "table_name") else {
            continue;
        };
        let Some(name) = str_at(r, "column_name") else {
            continue;
        };
        out.entry((None, table)).or_default().push(ColumnDesc {
            name,
            ty: str_at(r, "column_type").unwrap_or_default(),
            nullable: !bool_at(r, "not_null"),
            default_value: str_at(r, "default_value"),
            // `pk` is the 1-based position within the primary key, 0 when the
            // column is not part of it — not a boolean.
            primary_key: i64_at(r, "pk_ord").unwrap_or(0) > 0,
            position: i64_at(r, "cid").unwrap_or(0) as i32 + 1,
            foreign_key: None,
        });
    }

    merge_foreign_keys(state, db, filter, timeout_ms, &mut out).await?;
    Ok(out)
}

/// SQLite reports a foreign key's target column as NULL when the reference is
/// to the parent's primary key implicitly (`REFERENCES users` rather than
/// `REFERENCES users(id)`). Resolve that against the columns we already hold
/// rather than emitting a half-empty reference.
async fn merge_foreign_keys(
    state: &AppState,
    db: &str,
    filter: Option<&TableFilter>,
    timeout_ms: u64,
    columns: &mut HashMap<TableKey, Vec<ColumnDesc>>,
) -> Result<(), String> {
    let (clause, params) = filter_clause(filter);
    let sql = format!(
        "SELECT m.name AS table_name, f.\"from\" AS src_column, \
                f.\"table\" AS ref_table, f.\"to\" AS ref_column \
         FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) f \
         WHERE m.type = 'table' AND {NOT_INTERNAL}{clause}"
    );
    let rows = run(state, db, sql, params, timeout_ms).await?;

    // Primary keys of tables already loaded. Free, but only covers parents
    // inside the current filter — describing one table does not load its
    // parents, which is exactly when implicit references need resolving.
    let mut pk_of: HashMap<String, String> = columns
        .iter()
        .filter_map(|((_, t), cols)| {
            cols.iter()
                .find(|c| c.primary_key)
                .map(|c| (t.clone(), c.name.clone()))
        })
        .collect();

    // Look up any parent we are missing in one extra query rather than
    // dropping the reference. Dropping it would mean `describeTable` silently
    // reports no foreign key on a column that plainly has one.
    let unresolved: Vec<String> = rows
        .iter()
        .filter(|r| str_at(r, "ref_column").is_none())
        .filter_map(|r| str_at(r, "ref_table"))
        .filter(|t| !pk_of.contains_key(t))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if !unresolved.is_empty() {
        let placeholders = vec!["?"; unresolved.len()].join(", ");
        let sql = format!(
            "SELECT m.name AS table_name, p.name AS column_name \
             FROM sqlite_master m JOIN pragma_table_info(m.name) p \
             WHERE m.type = 'table' AND p.pk > 0 AND m.name IN ({placeholders}) \
             ORDER BY m.name, p.pk"
        );
        let params = unresolved.iter().cloned().map(Value::String).collect();
        for r in run(state, db, sql, params, timeout_ms).await? {
            let (Some(t), Some(c)) = (str_at(&r, "table_name"), str_at(&r, "column_name")) else {
                continue;
            };
            // Ordered by `p.pk`, so the first row per table is the leading
            // primary-key column — the one an implicit reference means.
            pk_of.entry(t).or_insert(c);
        }
    }

    for r in &rows {
        let (Some(table), Some(src), Some(ref_table)) = (
            str_at(r, "table_name"),
            str_at(r, "src_column"),
            str_at(r, "ref_table"),
        ) else {
            continue;
        };
        let ref_column = match str_at(r, "ref_column") {
            Some(c) => c,
            // Implicit `REFERENCES parent` means the parent's primary key.
            // Unresolvable only if the parent no longer exists.
            None => match pk_of.get(&ref_table) {
                Some(pk) => pk.clone(),
                None => continue,
            },
        };
        if let Some(cols) = columns.get_mut(&(None, table)) {
            if let Some(col) = cols.iter_mut().find(|c| c.name == src) {
                col.foreign_key = Some(ForeignKeyRef {
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
    let (clause, params) = filter_clause(filter);
    // `origin` is 'pk' for the implicit primary-key index, 'u' for a UNIQUE
    // constraint, 'c' for CREATE INDEX.
    let sql = format!(
        "SELECT m.name AS table_name, il.name AS index_name, \
                il.\"unique\" AS is_unique, il.origin AS origin, \
                ii.name AS column_name \
         FROM sqlite_master m \
         JOIN pragma_index_list(m.name) il \
         LEFT JOIN pragma_index_info(il.name) ii \
         WHERE m.type = 'table' AND {NOT_INTERNAL}{clause} \
         ORDER BY m.name, il.seq, ii.seqno"
    );
    let mut rows = run(state, db, sql, params, timeout_ms).await?;
    // `fold_index_rows` reads `is_primary`; sqlite spells that as origin='pk'.
    // Normalize in place so the fold stays a single pass.
    for r in rows.iter_mut() {
        let primary = str_at(r, "origin").as_deref() == Some("pk");
        r.insert("is_primary".to_string(), Value::Bool(primary));
    }
    Ok(fold_index_rows(&rows, |r| {
        str_at(r, "table_name").map(|t| (None, t))
    }))
}

/// SQLite has no cheap row estimate — `sqlite_stat1` exists only after an
/// explicit `ANALYZE`, and `COUNT(*)` is a full scan. Report nothing rather
/// than pay for a scan the caller did not ask for.
pub async fn row_estimates(
    _state: &AppState,
    _db: &str,
    _filter: Option<&TableFilter>,
    _timeout_ms: u64,
) -> Result<HashMap<TableKey, i64>, String> {
    Ok(HashMap::new())
}
