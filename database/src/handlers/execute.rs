//! `iii-database::execute` — write SQL.

use super::AppState;
use crate::driver;
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use crate::value::JsonParam;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct ExecuteReq {
    db: String,
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
    #[serde(default)]
    returning: Vec<String>,
}

pub async fn handle(state: &AppState, payload: Value) -> Result<Value, String> {
    let req: ExecuteReq = serde_json::from_value(payload).map_err(|e| {
        serde_json::to_string(&crate::error::DbError::InvalidParam {
            index: 0,
            reason: e.to_string(),
        })
        .unwrap_or_default()
    })?;

    let pool = state.pool(&req.db).map_err(err_to_str)?;
    // Reject empty SQL uniformly. See the matching guard in query.rs for why
    // this is at the handler boundary rather than per-driver: postgres' driver
    // accepts empty SQL as a no-op success, sqlite/mysql reject — guarding
    // here keeps the worker's contract symmetric across all three.
    if req.sql.trim().is_empty() {
        return Err(err_to_str(crate::error::DbError::DriverError {
            driver: format!("{:?}", pool.driver()),
            code: None,
            message: "empty SQL".into(),
            failed_index: None,
        }));
    }
    let params = JsonParam::from_json_slice(&req.params).map_err(err_to_str)?;

    let result = match pool {
        Pool::Sqlite(p) => driver::sqlite::execute(p, &req.sql, &params, &req.returning).await,
        Pool::Postgres(p) => driver::postgres::execute(p, &req.sql, &params, &req.returning).await,
        Pool::Mysql(p) => driver::mysql::execute(p, &req.sql, &params, &req.returning).await,
    }
    .map_err(err_to_str)?;

    let returned_rows =
        crate::handlers::query_rows_to_objects(&result.returned_columns, result.returned_rows);
    Ok(json!({
        "affected_rows": result.affected_rows,
        // serde renders Option<String>::None as JSON null; previously this
        // was unwrap_or_default() which produced the empty string "" and
        // forced callers to do "if last_insert_id" string truthiness checks.
        "last_insert_id": result.last_insert_id,
        "returned_rows": returned_rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::handlers::AppState;
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
    async fn execute_insert_returns_envelope() {
        let st = state();
        handle(
            &st,
            json!({
                "db": "primary",
                "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)"
            }),
        )
        .await
        .unwrap();

        let resp = handle(
            &st,
            json!({
                "db": "primary",
                "sql": "INSERT INTO t (n) VALUES (?)",
                "params": [42]
            }),
        )
        .await
        .unwrap();
        assert_eq!(resp["affected_rows"], 1);
        assert_eq!(resp["last_insert_id"], "1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_update_with_no_prior_insert_returns_null_last_insert_id() {
        // SQLite's `last_insert_rowid()` is sticky per-connection — it stays
        // set across non-INSERT statements until another INSERT runs. To
        // exercise the None branch we run an UPDATE against a freshly-created
        // table without any prior INSERT on this pool's connection.
        let st = state();
        handle(
            &st,
            json!({
                "db": "primary",
                "sql": "CREATE TABLE t (n INT)"
            }),
        )
        .await
        .unwrap();
        let resp = handle(
            &st,
            json!({
                "db": "primary",
                "sql": "UPDATE t SET n = ? WHERE n = ?",
                "params": [99, 1]
            }),
        )
        .await
        .unwrap();
        assert_eq!(resp["affected_rows"], 0);
        // No INSERT has ever run on this connection, so last_insert_rowid()
        // is 0 → driver returns None → JSON null (NOT the empty string "").
        assert!(
            resp["last_insert_id"].is_null(),
            "expected null, got {:?}",
            resp["last_insert_id"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_update_after_insert_does_not_carry_stale_last_insert_id() {
        // Regression: SQLite's last_insert_rowid() is sticky per-connection,
        // and the pool reuses connections. Without an is_insert() guard, an
        // UPDATE running on a connection whose prior caller ran an INSERT
        // would report the prior INSERT's rowid as last_insert_id — a phantom
        // success signal that corrupts caller logic.
        let st = state();
        handle(
            &st,
            json!({"db":"primary","sql":"CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)"}),
        )
        .await
        .unwrap();
        let ins = handle(
            &st,
            json!({"db":"primary","sql":"INSERT INTO t (n) VALUES (?)","params":[1]}),
        )
        .await
        .unwrap();
        assert_eq!(ins["last_insert_id"], "1");
        // Same pool, same connection (default max). The UPDATE must NOT
        // surface the rowid the INSERT just set.
        let upd = handle(
            &st,
            json!({"db":"primary","sql":"UPDATE t SET n = ? WHERE id = ?","params":[2, 1]}),
        )
        .await
        .unwrap();
        assert_eq!(upd["affected_rows"], 1);
        assert!(
            upd["last_insert_id"].is_null(),
            "UPDATE response leaked stale rowid: {:?}",
            upd["last_insert_id"]
        );
    }
}
