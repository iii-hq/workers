//! `database::listTables` / `describeTable` / `describeSchema` — what is
//! *inside* a database, as opposed to what is on the bus.
//!
//! The engine already introspects itself (`engine::functions::list` and
//! friends); nothing in the engine knows what a table is, because only this
//! worker holds the connection pools. These functions are the SQL-catalog
//! equivalent, so an agent can ask what tables exist and how they relate
//! without hand-writing `sqlite_master` / `information_schema` / `PRAGMA` per
//! driver — which is exactly what the console was doing before.
//!
//! `describe_table` is a one-table `describe_schema`, so there is a single
//! assembly path. The difference that matters is the filter: describing one
//! table scopes every catalog query to it, while describing a whole schema
//! runs each query once across all tables and regroups in Rust. A 200-table
//! schema therefore costs three or four queries, not six hundred.

use super::catalog::{self, ColumnDesc, IndexDesc, TableFilter, TableKey, TableKind, TableRef};
use super::AppState;
use crate::config::DriverKind;
use crate::error::DbError;
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_timeout() -> u64 {
    30_000
}

fn default_max_tables() -> usize {
    500
}

/// Hard ceiling regardless of what the caller asks for. Describing a schema
/// is bounded work; describing an unbounded one is not.
const MAX_TABLES_CEILING: usize = 2_000;

async fn driver_of(state: &AppState, db: &str) -> Result<DriverKind, String> {
    let pool = state.pool(db).await.map_err(err_to_str)?;
    Ok(match pool {
        Pool::Sqlite(_) => DriverKind::Sqlite,
        Pool::Postgres(_) => DriverKind::Postgres,
        Pool::Mysql(_) => DriverKind::Mysql,
    })
}

fn no_such_table(driver: DriverKind, table: &str) -> String {
    err_to_str(DbError::DriverError {
        driver: format!("{driver:?}").to_lowercase(),
        code: None,
        message: format!("no such table: {table}"),
        failed_index: None,
    })
}

/* ---------------- listTables ---------------- */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTablesReq {
    /// Logical database name. Optional — omitting it targets the sole
    /// configured database, or `primary` when several are configured.
    #[serde(default)]
    pub db: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListTablesResp {
    pub tables: Vec<TableRef>,
    pub count: usize,
}

pub async fn list_tables(state: &AppState, req: ListTablesReq) -> Result<ListTablesResp, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let tables = read_tables(state, &db, req.timeout_ms).await?;
    Ok(ListTablesResp {
        count: tables.len(),
        tables,
    })
}

async fn read_tables(state: &AppState, db: &str, timeout_ms: u64) -> Result<Vec<TableRef>, String> {
    match driver_of(state, db).await? {
        DriverKind::Sqlite => catalog::sqlite::list_tables(state, db, timeout_ms).await,
        DriverKind::Postgres => catalog::postgres::list_tables(state, db, timeout_ms).await,
        DriverKind::Mysql => catalog::mysql::list_tables(state, db, timeout_ms).await,
    }
}

/* ---------------- describeTable / describeSchema ---------------- */

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TableDescription {
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: TableKind,
    pub columns: Vec<ColumnDesc>,
    pub indexes: Vec<IndexDesc>,
    /// Planner estimate, never a `COUNT(*)`. Absent when the driver has no
    /// cheap estimate (sqlite) or has not analyzed the table yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count_estimate: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeTableReq {
    #[serde(default)]
    pub db: Option<String>,
    /// Table or view name. May be schema-qualified (`analytics.events`) on
    /// postgres; prefer the explicit `schema` field when the name itself
    /// contains a dot.
    pub table: String,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

pub async fn describe_table(
    state: &AppState,
    req: DescribeTableReq,
) -> Result<TableDescription, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let driver = driver_of(state, &db).await?;

    // Only postgres has a namespace above the table, so only there does a dot
    // in the name mean a schema qualifier.
    let (schema, table) = match (&req.schema, driver) {
        (Some(s), _) => (Some(s.clone()), req.table.clone()),
        (None, DriverKind::Postgres) => catalog::split_qualified(&req.table),
        (None, _) => (None, req.table.clone()),
    };

    let filter = TableFilter {
        schema: schema.clone(),
        table: table.clone(),
    };
    let mut described = assemble(state, &db, driver, Some(&filter), true, req.timeout_ms).await?;

    // Scoping the catalog queries to one table means an unknown name simply
    // yields nothing; say so rather than returning an empty description.
    if described.is_empty() {
        return Err(no_such_table(driver, &req.table));
    }
    Ok(described.remove(0))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeSchemaReq {
    #[serde(default)]
    pub db: Option<String>,
    /// Restrict to these tables. Omit for every table in the database.
    #[serde(default)]
    pub tables: Option<Vec<String>>,
    /// Indexes cost one extra catalog query. Off by default because the
    /// common caller (a relationship diagram) only needs columns and keys.
    #[serde(default)]
    pub include_indexes: bool,
    #[serde(default = "default_max_tables")]
    pub max_tables: usize,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DescribeSchemaResp {
    pub tables: Vec<TableDescription>,
    pub count: usize,
    /// True when `max_tables` cut the result short. Never silently truncate.
    pub truncated: bool,
}

pub async fn describe_schema(
    state: &AppState,
    req: DescribeSchemaReq,
) -> Result<DescribeSchemaResp, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let driver = driver_of(state, &db).await?;
    let limit = req.max_tables.min(MAX_TABLES_CEILING);

    let mut tables = assemble(
        state,
        &db,
        driver,
        None,
        req.include_indexes,
        req.timeout_ms,
    )
    .await?;

    if let Some(wanted) = &req.tables {
        let wanted: Vec<(Option<String>, String)> = wanted
            .iter()
            .map(|t| match driver {
                DriverKind::Postgres => catalog::split_qualified(t),
                _ => (None, t.clone()),
            })
            .collect();
        tables.retain(|d| {
            wanted
                .iter()
                .any(|(s, t)| &d.table == t && (s.is_none() || &d.schema == s))
        });
    }

    let truncated = tables.len() > limit;
    tables.truncate(limit);
    Ok(DescribeSchemaResp {
        count: tables.len(),
        tables,
        truncated,
    })
}

/// The single assembly path. One catalog query per aspect, regrouped by
/// table — never a query per table.
async fn assemble(
    state: &AppState,
    db: &str,
    driver: DriverKind,
    filter: Option<&TableFilter>,
    include_indexes: bool,
    timeout_ms: u64,
) -> Result<Vec<TableDescription>, String> {
    let tables = read_tables(state, db, timeout_ms).await?;

    let columns = match driver {
        DriverKind::Sqlite => catalog::sqlite::columns(state, db, filter, timeout_ms).await?,
        DriverKind::Postgres => catalog::postgres::columns(state, db, filter, timeout_ms).await?,
        DriverKind::Mysql => catalog::mysql::columns(state, db, filter, timeout_ms).await?,
    };

    let mut indexes: HashMap<TableKey, Vec<IndexDesc>> = HashMap::new();
    if include_indexes {
        indexes = match driver {
            DriverKind::Sqlite => catalog::sqlite::indexes(state, db, filter, timeout_ms).await?,
            DriverKind::Postgres => {
                catalog::postgres::indexes(state, db, filter, timeout_ms).await?
            }
            DriverKind::Mysql => catalog::mysql::indexes(state, db, filter, timeout_ms).await?,
        };
    }

    let estimates = match driver {
        DriverKind::Sqlite => catalog::sqlite::row_estimates(state, db, filter, timeout_ms).await?,
        DriverKind::Postgres => {
            catalog::postgres::row_estimates(state, db, filter, timeout_ms).await?
        }
        DriverKind::Mysql => catalog::mysql::row_estimates(state, db, filter, timeout_ms).await?,
    };

    let mut out = Vec::new();
    for t in tables {
        let key: TableKey = (t.schema.clone(), t.name.clone());
        // A table with no column rows was filtered out by the catalog query,
        // so it is not part of this result.
        let Some(mut cols) = columns.get(&key).cloned() else {
            continue;
        };
        cols.sort_by_key(|c| c.position);
        out.push(TableDescription {
            table: t.name,
            schema: t.schema,
            kind: t.kind,
            columns: cols,
            indexes: indexes.get(&key).cloned().unwrap_or_default(),
            row_count_estimate: estimates.get(&key).copied(),
        });
    }
    Ok(out)
}
