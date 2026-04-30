//! `iii-database::transaction` — atomic sequence of statements.

use super::AppState;
use crate::driver::{self, Isolation, TxStatement};
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use crate::value::JsonParam;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct TxReq {
    db: String,
    statements: Vec<TxStmtReq>,
    #[serde(default)]
    isolation: Option<String>,
}

#[derive(Deserialize)]
struct TxStmtReq {
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
}

pub async fn handle(state: &AppState, payload: Value) -> Result<Value, String> {
    let req: TxReq = serde_json::from_value(payload).map_err(|e| {
        serde_json::to_string(&crate::error::DbError::InvalidParam {
            index: 0,
            reason: e.to_string(),
        })
        .unwrap_or_default()
    })?;
    let pool = state.pool(&req.db).map_err(err_to_str)?;

    let isolation = match req.isolation.as_deref() {
        Some("read_committed") => Some(Isolation::ReadCommitted),
        Some("repeatable_read") => Some(Isolation::RepeatableRead),
        Some("serializable") => Some(Isolation::Serializable),
        Some(other) => {
            return Err(err_to_str(crate::error::DbError::InvalidParam {
                index: 0,
                reason: format!("unknown isolation `{other}`"),
            }))
        }
        None => None,
    };

    let mut stmts: Vec<TxStatement> = Vec::with_capacity(req.statements.len());
    for s in req.statements {
        let params = JsonParam::from_json_slice(&s.params).map_err(err_to_str)?;
        stmts.push(TxStatement { sql: s.sql, params });
    }

    let result = match pool {
        Pool::Sqlite(p) => driver::sqlite::transaction(p, stmts, isolation).await,
        Pool::Postgres(p) => driver::postgres::transaction(p, stmts, isolation).await,
        Pool::Mysql(p) => driver::mysql::transaction(p, stmts, isolation).await,
    };

    match result {
        Ok(steps) => Ok(json!({
            "committed": true,
            "results": steps.into_iter().map(|s| json!({
                "affected_rows": s.affected_rows,
                "rows": s.rows.into_iter()
                    .map(|r| r.0.into_iter().map(|v| v.into_json()).collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })),
        Err(e) => {
            let failed_index = match &e {
                crate::error::DbError::DriverError { failed_index, .. } => {
                    failed_index.unwrap_or(0)
                }
                _ => 0,
            };
            let error_value = serde_json::to_value(&e)
                .unwrap_or_else(|_| json!({"code": "DRIVER_ERROR", "message": e.to_string()}));
            Ok(json!({
                "committed": false,
                "failed_index": failed_index,
                "error": error_value,
            }))
        }
    }
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
    async fn tx_commits_when_all_succeed() {
        let st = state();
        crate::handlers::execute::handle(
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
                "statements": [
                    {"sql": "INSERT INTO t VALUES (?)", "params": [1]},
                    {"sql": "INSERT INTO t VALUES (?)", "params": [2]},
                ]
            }),
        )
        .await
        .unwrap();
        assert_eq!(resp["committed"], true);
        assert_eq!(resp["results"].as_array().unwrap().len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tx_returns_failed_index_on_rollback() {
        let st = state();
        crate::handlers::execute::handle(
            &st,
            json!({
                "db": "primary",
                "sql": "CREATE TABLE t (n INT NOT NULL)"
            }),
        )
        .await
        .unwrap();
        let resp = handle(
            &st,
            json!({
                "db": "primary",
                "statements": [
                    {"sql": "INSERT INTO t VALUES (?)", "params": [1]},
                    {"sql": "INSERT INTO t VALUES (?)", "params": [null]},
                ]
            }),
        )
        .await
        .unwrap();
        assert_eq!(resp["committed"], false);
        assert_eq!(resp["failed_index"], 1);
        assert!(
            resp["error"].is_object(),
            "error should be a structured object, got {:?}",
            resp["error"]
        );
        assert_eq!(resp["error"]["code"], "DRIVER_ERROR");
        assert_eq!(resp["error"]["driver"], "sqlite");
        assert_eq!(resp["error"]["failed_index"], 1);
    }
}
