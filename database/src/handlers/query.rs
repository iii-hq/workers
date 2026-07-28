//! `database::query` — read-only SQL.

use super::AppState;
use crate::driver::{self, ColumnMeta};
use crate::error::DbError;
use crate::pool::Pool;
use crate::value::JsonParam;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryReq {
    /// Logical database name. Optional — omitting it targets the sole
    /// configured database, or `primary` when several are configured.
    #[serde(default)]
    pub db: Option<String>,
    #[serde(alias = "query")]
    pub sql: String,
    #[serde(default, deserialize_with = "crate::handlers::lenient_params")]
    pub params: Vec<Value>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResp {
    pub rows: Vec<serde_json::Map<String, Value>>,
    pub row_count: usize,
    pub columns: Vec<ColumnMeta>,
}

fn default_timeout() -> u64 {
    30_000
}

/// Returns a JSON string body suitable to wrap in IIIError on failure.
pub async fn handle(state: &AppState, req: QueryReq) -> Result<QueryResp, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let pool = state.pool(&db).await.map_err(err_to_str)?;
    // Reject empty SQL uniformly. Postgres' tokio-postgres treats `client.query("")`
    // as a valid no-op and returns Ok([]), but sqlite (rusqlite) and mysql
    // (mysql_async) reject it at parse time — without this guard the worker's
    // contract diverges per driver.
    if req.sql.trim().is_empty() {
        return Err(err_to_str(DbError::DriverError {
            driver: format!("{:?}", pool.driver()),
            code: None,
            message: "empty SQL".into(),
            failed_index: None,
        }));
    }
    // A `query("BEGIN")` is just as pool-poisoning as an execute — sqlite
    // happily starts the transaction and returns zero rows.
    crate::handlers::reject_tx_control_sql(&req.sql).map_err(err_to_str)?;
    let params = JsonParam::from_json_slice(&req.params).map_err(err_to_str)?;

    let result = match &pool {
        Pool::Sqlite(p) => driver::sqlite::query(p, &req.sql, &params, req.timeout_ms).await,
        Pool::Postgres(p) => driver::postgres::query(p, &req.sql, &params, req.timeout_ms).await,
        Pool::Mysql(p) => driver::mysql::query(p, &req.sql, &params, req.timeout_ms).await,
    }
    .map_err(err_to_str)?;

    let row_count = result.rows.len();
    let rows = rows_to_objects(&result.columns, result.rows);
    Ok(QueryResp {
        rows,
        row_count,
        columns: result.columns,
    })
}

/// Project a result set into row-of-objects JSON. Consumes `rows` so each
/// `RowValue` cell can be moved into its `Value` form via `into_json` instead
/// of cloned — on a 1000-row × 10-col SELECT this removes ~10k allocations
/// of the cell payload data. Column names are still cloned per row because
/// `serde_json::Map` requires owned `String` keys; that's an unavoidable cost
/// of the row-of-objects shape and is dominated by the cell-data win.
pub(crate) fn rows_to_objects(
    columns: &[crate::driver::ColumnMeta],
    rows: Vec<crate::driver::Row>,
) -> Vec<serde_json::Map<String, Value>> {
    rows.into_iter()
        .map(|row| {
            let mut obj = serde_json::Map::with_capacity(columns.len());
            for (i, v) in row.0.into_iter().enumerate() {
                if let Some(col) = columns.get(i) {
                    obj.insert(col.name.clone(), v.into_json());
                }
            }
            obj
        })
        .collect()
}

pub(crate) fn err_to_str(e: DbError) -> String {
    serde_json::to_string(&e).unwrap_or_else(|_| {
        format!(
            "{{\"code\":\"DRIVER_ERROR\",\"message\":{:?}}}",
            e.to_string()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::pool::{Pool, SqlitePool};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state() -> AppState {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(RwLock::new(pools)),
            config: Arc::new(RwLock::new(crate::config::WorkerConfig::default())),
            handles: Arc::new(HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
        }
    }

    fn req(v: Value) -> QueryReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_returns_rows_envelope() {
        let st = state();
        if let Pool::Sqlite(p) = st.pool("primary").await.unwrap() {
            let c = p.acquire().await.unwrap();
            tokio::task::spawn_blocking(move || {
                c.with(|c| c.execute_batch("CREATE TABLE t (n INT); INSERT INTO t VALUES (1),(2);"))
            })
            .await
            .unwrap()
            .unwrap();
        }
        let resp = handle(
            &st,
            req(json!({"db":"primary","sql":"SELECT n FROM t ORDER BY n"})),
        )
        .await
        .unwrap();
        assert_eq!(resp.row_count, 2);
        assert_eq!(resp.rows[0]["n"], 1);
        assert_eq!(resp.columns[0].name, "n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_unknown_db_errors() {
        let st = state();
        let err = handle(&st, req(json!({"db":"missing","sql":"SELECT 1"})))
            .await
            .unwrap_err();
        assert!(err.contains("UNKNOWN_DB"), "got: {err}");
        // The error must enumerate the registered handles so a caller that
        // guessed a wrong name can self-correct from one failure.
        assert!(err.contains("\"available\":[\"primary\"]"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_rejects_transaction_control_sql() {
        // sqlite happily starts a transaction from a query("BEGIN") and
        // returns zero rows — the same pool poison as the execute path.
        let st = state();
        let err = handle(&st, req(json!({"db":"primary","sql":"BEGIN"})))
            .await
            .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("beginTransaction"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_omitted_db_defaults_to_sole_pool() {
        // LLM callers routinely omit `db` on their first call. With exactly
        // one configured pool the omission must resolve to it instead of
        // failing serde — the old `missing field db` error cost a wasted
        // round-trip in every live session.
        let st = state();
        let resp = handle(&st, req(json!({"sql":"SELECT 1 AS one"})))
            .await
            .unwrap();
        assert_eq!(resp.rows[0]["one"], 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn omitted_db_prefers_primary_among_many() {
        let st = state();
        {
            let extra = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
            st.pools
                .write()
                .await
                .insert("analytics".to_string(), Pool::Sqlite(extra));
        }
        assert_eq!(st.resolve_db(None).await.unwrap(), "primary");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn omitted_db_without_default_errors_with_available_list() {
        let st = state();
        {
            let mut pools = st.pools.write().await;
            let p = pools.remove("primary").unwrap();
            pools.insert("main".to_string(), p);
            let extra = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
            pools.insert("analytics".to_string(), Pool::Sqlite(extra));
        }
        let err = handle(&st, req(json!({"sql":"SELECT 1"})))
            .await
            .unwrap_err();
        assert!(err.contains("MISSING_DB"), "got: {err}");
        assert!(
            err.contains("\"available\":[\"analytics\",\"main\"]"),
            "got: {err}"
        );
    }

    #[test]
    fn request_schema_marks_db_optional() {
        let schema = serde_json::to_value(schemars::schema_for!(QueryReq).schema).unwrap();
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert!(
            !required.iter().any(|f| f == "db"),
            "db must not be required in the function schema: {required:?}"
        );
        assert!(required.iter().any(|f| f == "sql"));
    }
}
