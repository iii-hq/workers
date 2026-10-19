//! `database::distinctValues` — the values a column takes, most frequent
//! first. Feeds a filter chip's value picker: "status is one of …" offers
//! what is actually in the table instead of a blank field.
//!
//! One grouped read through `database::query` (so read-only, timed out and
//! pooled like every other read). `search` narrows by a case-insensitive
//! substring of the value's text form, matched literally (LIKE wildcards in
//! it are escaped).

use super::filter::{quote_ident, quote_table};
use super::query::{self, err_to_str, QueryReq};
use super::AppState;
use crate::config::DriverKind;
use crate::error::DbError;
use crate::pool::Pool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;

fn default_timeout() -> u64 {
    30_000
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DistinctValuesReq {
    #[serde(default)]
    pub db: Option<String>,
    pub table: String,
    /// Postgres schema; ignored elsewhere.
    #[serde(default)]
    pub schema: Option<String>,
    pub column: String,
    /// Keep values whose text form contains this, case-insensitively.
    #[serde(default)]
    pub search: Option<String>,
    /// Default 50, at most 500.
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DistinctValue {
    /// The value as JSON (`null` for NULL), the same encoding as query rows.
    pub value: Value,
    /// Rows holding it.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DistinctValuesResp {
    pub values: Vec<DistinctValue>,
    /// More distinct values exist past `limit`.
    pub truncated: bool,
    pub elapsed_ms: u64,
}

/// Escape LIKE metacharacters with `\`, matching the `ESCAPE` clause below.
fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The grouped read for `driver`. `$1` / `?` is the search pattern when
/// `search` is set.
pub(crate) fn build_sql(
    driver: DriverKind,
    schema: Option<&str>,
    table: &str,
    column: &str,
    search: bool,
    limit: u32,
) -> String {
    let target = quote_table(driver, schema, table);
    let col = quote_ident(driver, column);
    let predicate = match (search, driver) {
        (false, _) => String::new(),
        (true, DriverKind::Postgres) => format!(" WHERE CAST({col} AS TEXT) ILIKE $1 ESCAPE '\\'"),
        // MySQL reads `\` as an escape inside a literal: the escape
        // character is written `'\\'`.
        (true, DriverKind::Mysql) => {
            format!(" WHERE LOWER(CAST({col} AS CHAR)) LIKE LOWER(?) ESCAPE '\\\\'")
        }
        (true, DriverKind::Sqlite) => {
            format!(" WHERE LOWER(CAST({col} AS TEXT)) LIKE LOWER(?) ESCAPE '\\'")
        }
    };
    // One more than asked, to learn whether the list was cut.
    let fetch = limit + 1;
    format!(
        "SELECT {col} AS value, COUNT(*) AS count FROM {target}{predicate} \
         GROUP BY {col} ORDER BY COUNT(*) DESC, {col} LIMIT {fetch}"
    )
}

pub async fn handle(
    state: &AppState,
    req: DistinctValuesReq,
) -> Result<DistinctValuesResp, String> {
    for (field, value) in [("table", &req.table), ("column", &req.column)] {
        if value.trim().is_empty() {
            return Err(err_to_str(DbError::InvalidParam {
                index: 0,
                reason: format!("{field} is required"),
            }));
        }
    }
    let db = state.resolve_db(req.db.clone()).await.map_err(err_to_str)?;
    let driver = match state.pool(&db).await.map_err(err_to_str)? {
        Pool::Sqlite(_) => DriverKind::Sqlite,
        Pool::Postgres(_) => DriverKind::Postgres,
        Pool::Mysql(_) => DriverKind::Mysql,
    };
    let limit = match req.limit {
        None => DEFAULT_LIMIT,
        Some(0) => {
            return Err(err_to_str(DbError::InvalidParam {
                index: 0,
                reason: format!("limit must be 1-{MAX_LIMIT}"),
            }))
        }
        Some(n) => n.min(MAX_LIMIT),
    };
    let search = req
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let sql = build_sql(
        driver,
        req.schema.as_deref(),
        &req.table,
        &req.column,
        search.is_some(),
        limit,
    );
    let params = search
        .map(|s| vec![Value::String(format!("%{}%", escape_like(s)))])
        .unwrap_or_default();
    let resp = query::handle(
        state,
        QueryReq {
            db: Some(db),
            sql,
            params,
            timeout_ms: req.timeout_ms,
            record_history: false,
            max_rows: None,
            query_only: false,
        },
    )
    .await?;
    let mut values: Vec<DistinctValue> = resp
        .rows
        .into_iter()
        .map(|mut row| DistinctValue {
            value: row.remove("value").unwrap_or(Value::Null),
            count: match row.remove("count") {
                Some(Value::Number(n)) => n.as_i64().unwrap_or_default(),
                Some(Value::String(s)) => s.parse().unwrap_or_default(),
                _ => 0,
            },
        })
        .collect();
    let truncated = values.len() > limit as usize;
    values.truncate(limit as usize);
    Ok(DistinctValuesResp {
        values,
        truncated,
        elapsed_ms: resp.elapsed_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::pool::SqlitePool;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    async fn state() -> AppState {
        let pool = SqlitePool::new(
            "sqlite::memory:",
            &PoolConfig {
                max: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let c = pool.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            c.with(|c| {
                c.execute_batch(
                    "CREATE TABLE t (status TEXT, n INT);
                     INSERT INTO t VALUES ('open',1),('open',2),('held',3),('Open%',4),(NULL,5),('open',6);",
                )
            })
        })
        .await
        .unwrap()
        .unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(RwLock::new(pools)),
            config: Arc::new(RwLock::new(crate::config::WorkerConfig::default())),
            handles: Arc::new(HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
            runtime: Default::default(),
        }
    }

    fn req(v: Value) -> DistinctValuesReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn most_frequent_first_with_null_and_truncation() {
        let st = state().await;
        let resp = handle(
            &st,
            req(json!({"table": "t", "column": "status", "limit": 2})),
        )
        .await
        .unwrap();
        assert_eq!(resp.values[0].value, json!("open"));
        assert_eq!(resp.values[0].count, 3);
        assert_eq!(resp.values.len(), 2);
        assert!(resp.truncated);

        let all = handle(&st, req(json!({"table": "t", "column": "status"})))
            .await
            .unwrap();
        assert!(!all.truncated);
        assert!(all.values.iter().any(|v| v.value.is_null()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_is_case_insensitive_and_literal() {
        let st = state().await;
        let resp = handle(
            &st,
            req(json!({"table": "t", "column": "status", "search": "OPEN"})),
        )
        .await
        .unwrap();
        let got: Vec<&Value> = resp.values.iter().map(|v| &v.value).collect();
        assert!(
            got.contains(&&json!("open")) && got.contains(&&json!("Open%")),
            "{got:?}"
        );
        // `%` matches literally, not as a wildcard.
        let resp = handle(
            &st,
            req(json!({"table": "t", "column": "status", "search": "%"})),
        )
        .await
        .unwrap();
        assert_eq!(resp.values.len(), 1);
        assert_eq!(resp.values[0].value, json!("Open%"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_zero_limit_is_refused_and_a_huge_one_is_capped() {
        let st = state().await;
        let err = handle(
            &st,
            req(json!({"table": "t", "column": "status", "limit": 0})),
        )
        .await
        .unwrap_err();
        assert!(err.contains("limit must be 1-500"), "{err}");
        let resp = handle(
            &st,
            req(json!({"table": "t", "column": "status", "limit": 100000})),
        )
        .await
        .unwrap();
        assert!(!resp.truncated);
    }

    #[test]
    fn identifiers_are_quoted_per_driver() {
        let sql = build_sql(DriverKind::Mysql, None, "t`x", "c", true, 10);
        assert!(sql.contains("`t``x`"), "{sql}");
        assert!(sql.contains("ESCAPE '\\\\'"), "{sql}");
        let sql = build_sql(DriverKind::Postgres, Some("s"), "t", "c\"", false, 10);
        assert!(
            sql.contains(r#""s"."t""#) && sql.contains(r#""c""""#),
            "{sql}"
        );
        assert!(sql.ends_with("LIMIT 11"));
    }
}
