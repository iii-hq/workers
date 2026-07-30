//! Driver-neutral catalog shapes and the per-driver readers behind
//! `database::listTables` / `describeTable` / `describeSchema`.
//!
//! Every reader goes through `database::query`, so catalog reads inherit the
//! same pool, timeout, and read-only-transaction handling as user SQL.
//!
//! Two rules hold across all three drivers:
//!
//! 1. **No array-valued columns.** `RowValue` has no array variant, so a
//!    catalog query returning `text[]`/`int[]` fails to decode. Anywhere a
//!    catalog exposes an array (postgres `indkey`, `most_common_vals`), it is
//!    unnested into scalar rows and regrouped in Rust.
//! 2. **One query per aspect, not per table.** The readers take an optional
//!    table filter; `describeSchema` omits it and regroups, so a 200-table
//!    schema costs three queries rather than six hundred.

pub mod mysql;
pub mod postgres;
pub mod sqlite;

use super::AppState;
use crate::handlers::query::{self, QueryReq};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TableKind {
    Table,
    View,
}

/// A relation. `schema` is populated only where the driver has a meaningful
/// namespace above the table (postgres); it is never concatenated into `name`,
/// because `analytics.events` and a table literally called `analytics.events`
/// are different things and callers must be able to tell them apart.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TableRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: TableKind,
}

/// Where a foreign key points. Structured rather than a `"table.column"`
/// string so a schema-qualified target stays unambiguous.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ForeignKeyRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ColumnDesc {
    pub name: String,
    /// Driver-reported type text (`TEXT`, `integer`, `varchar(255)`).
    #[serde(rename = "type")]
    pub ty: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    pub primary_key: bool,
    /// 1-based ordinal, as the driver reports it.
    pub position: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreign_key: Option<ForeignKeyRef>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IndexDesc {
    pub name: String,
    pub unique: bool,
    pub primary: bool,
    /// Indexed columns in ordinal order. Empty when the index is on an
    /// expression rather than plain columns.
    pub columns: Vec<String>,
}

/// Key a per-table result is grouped under. `describeSchema` fans one query
/// out across every table, so each row has to say which table it belongs to.
pub type TableKey = (Option<String>, String);

/// Identifies one table for the readers. `schema` is ignored by sqlite and
/// mysql, which have no namespace above the table within a connection.
#[derive(Debug, Clone)]
pub struct TableFilter {
    pub schema: Option<String>,
    pub table: String,
}

/// Split a possibly schema-qualified name into `(schema, bare)`. Only applied
/// where the driver actually has schemas — see `TableFilter`.
pub fn split_qualified(name: &str) -> (Option<String>, String) {
    match name.split_once('.') {
        Some((schema, bare)) if !schema.is_empty() && !bare.is_empty() => {
            (Some(schema.to_string()), bare.to_string())
        }
        _ => (None, name.to_string()),
    }
}

/// Double-quote an identifier, doubling embedded quotes. Used only where a
/// value cannot be bound as a parameter.
pub fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Run a catalog statement through the ordinary query path.
pub async fn run(
    state: &AppState,
    db: &str,
    sql: impl Into<String>,
    params: Vec<Value>,
    timeout_ms: u64,
) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    let resp = query::handle(
        state,
        QueryReq {
            db: Some(db.to_string()),
            sql: sql.into(),
            params,
            timeout_ms,
            // Catalog reads are the page's plumbing, not the user's queries.
            record_history: false,
        },
    )
    .await?;
    Ok(resp.rows)
}

type Row = serde_json::Map<String, Value>;

pub fn str_at(row: &Row, key: &str) -> Option<String> {
    match row.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

pub fn i64_at(row: &Row, key: &str) -> Option<i64> {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

/// Catalogs are inconsistent about how they spell a boolean: postgres returns
/// a real bool, mysql returns 0/1, sqlite's PRAGMAs return 0/1, and
/// `information_schema` returns the strings `YES`/`NO`. Normalize all of them.
pub fn bool_at(row: &Row, key: &str) -> bool {
    match row.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().is_some_and(|v| v != 0),
        Some(Value::String(s)) => {
            matches!(s.to_ascii_lowercase().as_str(), "yes" | "true" | "t" | "1")
        }
        _ => false,
    }
}

/// Group unnested index rows into `IndexDesc`, preserving first-seen order so
/// the output is stable across runs.
pub fn fold_index_rows<F>(rows: &[Row], key_of: F) -> HashMap<TableKey, Vec<IndexDesc>>
where
    F: Fn(&Row) -> Option<TableKey>,
{
    let mut out: HashMap<TableKey, Vec<IndexDesc>> = HashMap::new();
    for row in rows {
        let Some(key) = key_of(row) else { continue };
        let Some(name) = str_at(row, "index_name") else {
            continue;
        };
        let entry = out.entry(key).or_default();
        let idx = match entry.iter_mut().find(|i| i.name == name) {
            Some(existing) => existing,
            None => {
                entry.push(IndexDesc {
                    name: name.clone(),
                    unique: bool_at(row, "is_unique"),
                    primary: bool_at(row, "is_primary"),
                    columns: Vec::new(),
                });
                entry.last_mut().expect("just pushed")
            }
        };
        if let Some(col) = str_at(row, "column_name") {
            idx.columns.push(col);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_qualified_handles_bare_and_qualified_names() {
        assert_eq!(split_qualified("users"), (None, "users".into()));
        assert_eq!(
            split_qualified("analytics.events"),
            (Some("analytics".into()), "events".into())
        );
        // Degenerate halves stay whole rather than producing an empty schema.
        assert_eq!(split_qualified(".events"), (None, ".events".into()));
        assert_eq!(split_qualified("analytics."), (None, "analytics.".into()));
    }

    #[test]
    fn quote_ident_doubles_embedded_quotes() {
        assert_eq!(quote_ident("users"), "\"users\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn bool_at_normalizes_every_catalog_spelling() {
        let row: Row = serde_json::from_str(
            r#"{"a": true, "b": 1, "c": 0, "d": "YES", "e": "NO", "f": "t", "g": null}"#,
        )
        .unwrap();
        assert!(bool_at(&row, "a"));
        assert!(bool_at(&row, "b"));
        assert!(!bool_at(&row, "c"));
        assert!(bool_at(&row, "d"));
        assert!(!bool_at(&row, "e"));
        assert!(bool_at(&row, "f"));
        assert!(!bool_at(&row, "g"));
        assert!(!bool_at(&row, "missing"));
    }

    #[test]
    fn i64_at_accepts_the_string_counts_mysql_returns() {
        let row: Row =
            serde_json::from_str(r#"{"n": 42, "s": "1234", "f": 7.0, "x": "no"}"#).unwrap();
        assert_eq!(i64_at(&row, "n"), Some(42));
        assert_eq!(i64_at(&row, "s"), Some(1234));
        assert_eq!(i64_at(&row, "f"), Some(7));
        assert_eq!(i64_at(&row, "x"), None);
    }

    #[test]
    fn fold_index_rows_groups_columns_in_ordinal_order() {
        let rows: Vec<Row> = serde_json::from_str(
            r#"[
              {"table_name":"orders","index_name":"pk_orders","is_unique":true,"is_primary":true,"column_name":"id"},
              {"table_name":"orders","index_name":"ix_o","is_unique":false,"is_primary":false,"column_name":"user_id"},
              {"table_name":"orders","index_name":"ix_o","is_unique":false,"is_primary":false,"column_name":"created_at"}
            ]"#,
        )
        .unwrap();
        let folded = fold_index_rows(&rows, |r| str_at(r, "table_name").map(|t| (None, t)));
        let idxs = &folded[&(None, "orders".to_string())];
        assert_eq!(idxs.len(), 2);
        assert_eq!(idxs[0].name, "pk_orders");
        assert!(idxs[0].primary && idxs[0].unique);
        assert_eq!(idxs[1].columns, vec!["user_id", "created_at"]);
    }
}
