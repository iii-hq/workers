//! `iii-database::query` — read-only SQL.

use super::AppState;
use crate::driver;
use crate::error::DbError;
use crate::pool::Pool;
use crate::value::JsonParam;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct QueryReq {
    db: String,
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30_000
}

/// Returns a JSON string body suitable to wrap in IIIError on failure.
pub async fn handle(state: &AppState, payload: Value) -> Result<Value, String> {
    let req: QueryReq = serde_json::from_value(payload).map_err(|e| {
        serde_json::to_string(&DbError::InvalidParam {
            index: 0,
            reason: e.to_string(),
        })
        .unwrap_or_default()
    })?;

    let pool = state.pool(&req.db).map_err(err_to_str)?;
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
    let params = JsonParam::from_json_slice(&req.params).map_err(err_to_str)?;

    let result = match pool {
        Pool::Sqlite(p) => driver::sqlite::query(p, &req.sql, &params, req.timeout_ms).await,
        Pool::Postgres(p) => driver::postgres::query(p, &req.sql, &params, req.timeout_ms).await,
        Pool::Mysql(p) => driver::mysql::query(p, &req.sql, &params, req.timeout_ms).await,
    }
    .map_err(err_to_str)?;

    let row_count = result.rows.len();
    let rows_json = rows_to_objects(&result.columns, result.rows);
    Ok(json!({
        "rows": rows_json,
        "row_count": row_count,
        "columns": result.columns,
    }))
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

    fn state() -> AppState {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(pools),
            handles: Arc::new(HandleRegistry::new()),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_returns_rows_envelope() {
        let st = state();
        if let Pool::Sqlite(p) = st.pool("primary").unwrap() {
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
            json!({"db":"primary","sql":"SELECT n FROM t ORDER BY n"}),
        )
        .await
        .unwrap();
        assert_eq!(resp["row_count"], 2);
        assert_eq!(resp["rows"][0]["n"], 1);
        assert_eq!(resp["columns"][0]["name"], "n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_unknown_db_errors() {
        let st = state();
        let err = handle(&st, json!({"db":"missing","sql":"SELECT 1"}))
            .await
            .unwrap_err();
        assert!(err.contains("UNKNOWN_DB"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_missing_db_field_errors() {
        let st = state();
        let err = handle(&st, json!({"sql":"SELECT 1"})).await.unwrap_err();
        assert!(
            err.contains("INVALID_PARAM") || err.contains("missing"),
            "got: {err}"
        );
    }
}
