//! `database::browseTable` — paged, sorted, filtered reads without the caller
//! writing SQL.
//!
//! Subsumes three things the console used to do by hand: building
//! `SELECT * … ORDER BY … LIMIT/OFFSET`, running a matching `COUNT(*)`, and
//! compiling filter chips into a `WHERE`. It also covers foreign-key lookup —
//! that is an equality filter at `page_size: 1` — so there is no separate
//! read-a-row function.

use super::filter::{self, FilterSpec, SortSpec};
use super::query::{self, err_to_str, QueryReq};
use super::AppState;
use crate::config::DriverKind;
use crate::driver::ColumnMeta;
use crate::error::DbError;
use crate::pool::Pool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_page_size() -> u32 {
    50
}

fn default_timeout() -> u64 {
    30_000
}

fn default_include_total() -> bool {
    true
}

/// Ceiling on a single page. A caller wanting everything should page.
const MAX_PAGE_SIZE: u32 = 1_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowseTableReq {
    #[serde(default)]
    pub db: Option<String>,
    pub table: String,
    #[serde(default)]
    pub schema: Option<String>,
    /// Zero-based.
    #[serde(default)]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    /// Applied in order; sort priority is position in the list.
    #[serde(default)]
    pub sort: Vec<SortSpec>,
    /// Combined with AND.
    #[serde(default)]
    pub filters: Vec<FilterSpec>,
    /// A filtered `COUNT(*)` is a second query and can be expensive on a
    /// large table. Turn it off while the caller is still typing.
    #[serde(default = "default_include_total")]
    pub include_total: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BrowseTableResp {
    pub rows: Vec<serde_json::Map<String, Value>>,
    pub columns: Vec<ColumnMeta>,
    pub page: u32,
    pub page_size: u32,
    /// Derived from a sentinel row, so it is correct without a count.
    pub has_more: bool,
    /// Total matching the same filters. Absent when not requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
}

async fn driver_of(state: &AppState, db: &str) -> Result<DriverKind, String> {
    Ok(match state.pool(db).await.map_err(err_to_str)? {
        Pool::Sqlite(_) => DriverKind::Sqlite,
        Pool::Postgres(_) => DriverKind::Postgres,
        Pool::Mysql(_) => DriverKind::Mysql,
    })
}

pub async fn handle(state: &AppState, req: BrowseTableReq) -> Result<BrowseTableResp, String> {
    if req.table.trim().is_empty() {
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "table is required".into(),
        }));
    }
    let db = state.resolve_db(req.db.clone()).await.map_err(err_to_str)?;
    let driver = driver_of(state, &db).await?;

    let page_size = req.page_size.clamp(1, MAX_PAGE_SIZE);
    let offset = (req.page as u64) * (page_size as u64);

    let target = filter::quote_table(driver, req.schema.as_deref(), &req.table);
    let where_clause = filter::compile_where(driver, &req.filters, 1).map_err(err_to_str)?;
    let predicate = if where_clause.sql.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clause.sql)
    };

    let order = filter::compile_order_by(driver, &req.sort).map_err(err_to_str)?;
    let order_by = if order.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {order}")
    };

    // Fetch one extra row to learn whether another page exists, so `has_more`
    // is right even when the caller skipped the count.
    let limit = page_size as u64 + 1;
    let sql = format!("SELECT * FROM {target}{predicate}{order_by} LIMIT {limit} OFFSET {offset}");

    let mut resp = query::handle(
        state,
        QueryReq {
            db: Some(db.clone()),
            sql,
            params: where_clause.params.clone(),
            timeout_ms: req.timeout_ms,
            record_history: false,
        },
    )
    .await?;

    let has_more = resp.rows.len() as u64 > page_size as u64;
    resp.rows.truncate(page_size as usize);

    let total = if req.include_total {
        let count_sql = format!("SELECT COUNT(*) AS total FROM {target}{predicate}");
        let counted = query::handle(
            state,
            QueryReq {
                db: Some(db),
                sql: count_sql,
                params: where_clause.params,
                timeout_ms: req.timeout_ms,
                record_history: false,
            },
        )
        .await?;
        counted
            .rows
            .first()
            .and_then(|r| r.get("total"))
            .and_then(|v| match v {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => s.parse().ok(),
                _ => None,
            })
    } else {
        None
    };

    Ok(BrowseTableResp {
        rows: resp.rows,
        columns: resp.columns,
        page: req.page,
        page_size,
        has_more,
        total,
    })
}
