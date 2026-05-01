//! `iii-database::transaction` — atomic sequence of statements.

use super::AppState;
use crate::driver::{self, Isolation, TxStatement};
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use crate::value::JsonParam;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, JsonSchema)]
pub struct TxReq {
    pub db: String,
    pub statements: Vec<TxStmtReq>,
    #[serde(default)]
    pub isolation: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TxStmtReq {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct TxStepResp {
    pub affected_rows: u64,
    pub rows: Vec<Vec<Value>>,
}

#[derive(Serialize, JsonSchema)]
pub struct TxResp {
    pub committed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<TxStepResp>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

pub async fn handle(state: &AppState, req: TxReq) -> Result<TxResp, String> {
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
        Ok(steps) => Ok(TxResp {
            committed: true,
            results: Some(
                steps
                    .into_iter()
                    .map(|s| TxStepResp {
                        affected_rows: s.affected_rows,
                        rows: s
                            .rows
                            .into_iter()
                            .map(|r| r.0.into_iter().map(|v| v.into_json()).collect::<Vec<_>>())
                            .collect::<Vec<_>>(),
                    })
                    .collect(),
            ),
            failed_index: None,
            error: None,
        }),
        Err(e) => {
            let failed_index = match &e {
                crate::error::DbError::DriverError { failed_index, .. } => {
                    failed_index.unwrap_or(0)
                }
                _ => 0,
            };
            let error_value = serde_json::to_value(&e)
                .unwrap_or_else(|_| json!({"code": "DRIVER_ERROR", "message": e.to_string()}));
            Ok(TxResp {
                committed: false,
                results: None,
                failed_index: Some(failed_index),
                error: Some(error_value),
            })
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

    fn tx_req(v: Value) -> TxReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tx_commits_when_all_succeed() {
        let st = state();
        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(json!({
                "db": "primary",
                "sql": "CREATE TABLE t (n INT)"
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let resp = handle(
            &st,
            tx_req(json!({
                "db": "primary",
                "statements": [
                    {"sql": "INSERT INTO t VALUES (?)", "params": [1]},
                    {"sql": "INSERT INTO t VALUES (?)", "params": [2]},
                ]
            })),
        )
        .await
        .unwrap();
        assert!(resp.committed);
        assert_eq!(resp.results.as_ref().unwrap().len(), 2);
        assert!(resp.failed_index.is_none());
        assert!(resp.error.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tx_returns_failed_index_on_rollback() {
        let st = state();
        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(json!({
                "db": "primary",
                "sql": "CREATE TABLE t (n INT NOT NULL)"
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let resp = handle(
            &st,
            tx_req(json!({
                "db": "primary",
                "statements": [
                    {"sql": "INSERT INTO t VALUES (?)", "params": [1]},
                    {"sql": "INSERT INTO t VALUES (?)", "params": [null]},
                ]
            })),
        )
        .await
        .unwrap();
        assert!(!resp.committed);
        assert_eq!(resp.failed_index, Some(1));
        let err = resp.error.as_ref().expect("error should be present");
        assert!(
            err.is_object(),
            "error should be a structured object, got {err:?}"
        );
        assert_eq!(err["code"], "DRIVER_ERROR");
        assert_eq!(err["driver"], "sqlite");
        assert_eq!(err["failed_index"], 1);
        assert!(resp.results.is_none());
    }

    #[test]
    fn tx_resp_skips_none_fields_on_wire() {
        // Wire-format invariant: success shape has no `failed_index`/`error`,
        // failure shape has no `results`. Verifies the
        // skip_serializing_if = "Option::is_none" attributes are wired up.
        let success = TxResp {
            committed: true,
            results: Some(vec![]),
            failed_index: None,
            error: None,
        };
        let v = serde_json::to_value(&success).unwrap();
        assert!(v.get("failed_index").is_none());
        assert!(v.get("error").is_none());
        assert!(v.get("results").is_some());

        let failure = TxResp {
            committed: false,
            results: None,
            failed_index: Some(0),
            error: Some(json!({"code": "DRIVER_ERROR"})),
        };
        let v = serde_json::to_value(&failure).unwrap();
        assert!(v.get("results").is_none());
        assert!(v.get("failed_index").is_some());
        assert!(v.get("error").is_some());
    }
}
