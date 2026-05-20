//! `database::rollbackTransaction` — finalize an interactive transaction
//! by issuing `ROLLBACK`. Like `commitTransaction`, takes the entry out of
//! the registry first so concurrent statement calls fast-fail with
//! `TRANSACTION_NOT_FOUND`.

use super::AppState;
use crate::driver;
use crate::handle::PinnedConn;
use crate::handlers::query::err_to_str;
use crate::transaction::driver_system;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
pub struct RollbackTxReq {
    pub transaction_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RollbackTxResp {
    pub rolled_back: bool,
}

pub async fn handle(state: &AppState, req: RollbackTxReq) -> Result<RollbackTxResp, String> {
    let taken = match state.transactions.take(&req.transaction_id).await {
        Ok(t) => t,
        Err(e) => {
            state.log.warn(
                "db_tx_unknown",
                Some(json!({
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "ROLLBACK",
                })),
            );
            return Err(err_to_str(e));
        }
    };

    let started_at = taken.started_at;
    let driver = taken.driver;
    let db_name = taken.db_name.clone();
    let mut guard = taken.conn_arc.lock_owned().await;

    let result = match &mut *guard {
        PinnedConn::Sqlite(slot) => driver::sqlite::tx_rollback(slot).await,
        PinnedConn::Postgres(client) => driver::postgres::tx_rollback(client).await,
        PinnedConn::Mysql(conn) => driver::mysql::tx_rollback(conn).await,
    };

    let duration_ms = (chrono::Utc::now() - started_at).num_milliseconds().max(0);

    match result {
        Ok(()) => {
            state.log.info(
                "db_tx_rolled_back",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.operation": "ROLLBACK",
                    "db.transaction.id": req.transaction_id,
                    "duration_ms": duration_ms,
                    "reason": "explicit",
                })),
            );
            Ok(RollbackTxResp { rolled_back: true })
        }
        Err(e) => {
            state.log.error(
                "db_tx_rollback_failed",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.operation": "ROLLBACK",
                    "db.transaction.id": req.transaction_id,
                    "duration_ms": duration_ms,
                    "error": serde_json::to_value(&e).ok(),
                })),
            );
            Err(err_to_str(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::begin_transaction::tests::state;
    use serde_json::{json, Value};

    fn req(v: Value) -> RollbackTxReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rollback_returns_rolled_back_true_and_removes_from_registry() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let resp = handle(&st, req(json!({ "transaction_id": begin.transaction.id })))
            .await
            .unwrap();
        assert!(resp.rolled_back);
        assert_eq!(st.transactions.len().await, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rollback_unknown_id_returns_transaction_not_found() {
        let st = state();
        let err = handle(
            &st,
            req(json!({ "transaction_id": "00000000-0000-0000-0000-000000000000" })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("TRANSACTION_NOT_FOUND"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn writes_inside_rolled_back_tx_are_not_visible() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let url = format!("sqlite:{}", tmp.path().display());
        let pool =
            crate::pool::SqlitePool::new(&url, &crate::config::PoolConfig::default()).unwrap();
        let mut pools = std::collections::HashMap::new();
        pools.insert("primary".to_string(), crate::pool::Pool::Sqlite(pool));
        let st = crate::handlers::AppState {
            pools: std::sync::Arc::new(pools),
            handles: std::sync::Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_sdk::Logger::new(),
        };

        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary", "sql": "CREATE TABLE t (n INT)" }))
                .unwrap(),
        )
        .await
        .unwrap();

        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let id = begin.transaction.id;

        crate::handlers::transaction_execute::handle(
            &st,
            serde_json::from_value(json!({
                "transaction_id": id.clone(),
                "sql": "INSERT INTO t (n) VALUES (?)",
                "params": [99]
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        handle(&st, req(json!({ "transaction_id": id })))
            .await
            .unwrap();

        let q = crate::handlers::query::handle(
            &st,
            serde_json::from_value(
                json!({ "db": "primary", "sql": "SELECT COUNT(*) AS c FROM t" }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(q.rows[0]["c"], 0);
    }
}
