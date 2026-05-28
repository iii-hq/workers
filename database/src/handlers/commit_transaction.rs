//! `database::commitTransaction` — finalize an interactive transaction
//! by issuing `COMMIT`. Removes the entry from the registry before locking
//! the connection so concurrent `transactionQuery` / `transactionExecute`
//! calls against the same id fast-fail with `TRANSACTION_NOT_FOUND` once
//! finalization is in progress.

use super::AppState;
use crate::driver;
use crate::handle::PinnedConn;
use crate::handlers::query::err_to_str;
use crate::transaction::driver_system;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
pub struct CommitTxReq {
    pub transaction_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommitTxResp {
    pub committed: bool,
}

pub async fn handle(state: &AppState, req: CommitTxReq) -> Result<CommitTxResp, String> {
    let taken = match state.transactions.take(&req.transaction_id).await {
        Ok(t) => t,
        Err(e) => {
            state.log.warn(
                "db_tx_unknown",
                Some(json!({
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "COMMIT",
                })),
            );
            return Err(err_to_str(e));
        }
    };

    let started_at = taken.started_at;
    let driver = taken.driver;
    let db_name = taken.db_name.clone();
    // Lock the conn — queues behind any in-flight transactionQuery/Execute.
    // The Arc's strong count goes to 1 here (registry has dropped its ref),
    // so when the guard drops at the end of this function the PinnedConn
    // drops and the underlying connection returns to its pool.
    let mut guard = taken.conn_arc.lock_owned().await;

    let result = match &mut *guard {
        PinnedConn::Sqlite(slot) => driver::sqlite::tx_commit(slot).await,
        PinnedConn::Postgres(client) => driver::postgres::tx_commit(client).await,
        PinnedConn::Mysql(conn) => driver::mysql::tx_commit(conn).await,
    };

    let duration_ms = (chrono::Utc::now() - started_at).num_milliseconds().max(0);

    match result {
        Ok(()) => {
            state.log.info(
                "db_tx_committed",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.operation": "COMMIT",
                    "db.transaction.id": req.transaction_id,
                    "duration_ms": duration_ms,
                })),
            );
            Ok(CommitTxResp { committed: true })
        }
        Err(e) => {
            // COMMIT failed — issue best-effort ROLLBACK to leave the
            // connection in a known state before the pool's recycler
            // (Postgres deadpool uses Fast recycle, which does NOT issue
            // ROLLBACK) hands it to the next caller. Without this, the next
            // pool consumer can see "current transaction is aborted,
            // commands ignored" until the conn is closed.
            let rb = match &mut *guard {
                PinnedConn::Sqlite(slot) => driver::sqlite::tx_rollback(slot).await,
                PinnedConn::Postgres(client) => driver::postgres::tx_rollback(client).await,
                PinnedConn::Mysql(conn) => driver::mysql::tx_rollback(conn).await,
            };
            state.log.error(
                "db_tx_commit_failed",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.operation": "COMMIT",
                    "db.transaction.id": req.transaction_id,
                    "duration_ms": duration_ms,
                    "rollback_after_commit_failure": rb.is_ok(),
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

    fn req(v: Value) -> CommitTxReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_returns_committed_true_and_removes_from_registry() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let id = begin.transaction.id;
        let resp = handle(&st, req(json!({ "transaction_id": id.clone() })))
            .await
            .unwrap();
        assert!(resp.committed);
        assert_eq!(st.transactions.len().await, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_twice_returns_transaction_not_found() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let id = begin.transaction.id;
        handle(&st, req(json!({ "transaction_id": id.clone() })))
            .await
            .unwrap();
        let err = handle(&st, req(json!({ "transaction_id": id })))
            .await
            .unwrap_err();
        assert!(err.contains("TRANSACTION_NOT_FOUND"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn writes_committed_via_interactive_tx_are_visible_after_commit() {
        // Use an on-disk SQLite so the txn's pinned conn and the post-commit
        // verification share a single database file. In-memory SQLite gives
        // each connection its own separate database.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let url = format!("sqlite:{}", tmp.path().display());
        let pool =
            crate::pool::SqlitePool::new(&url, &crate::config::PoolConfig::default()).unwrap();
        let mut pools = std::collections::HashMap::new();
        pools.insert("primary".to_string(), crate::pool::Pool::Sqlite(pool));
        let st = crate::handlers::AppState {
            pools: std::sync::Arc::new(tokio::sync::RwLock::new(pools)),
            handles: std::sync::Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_observability::Logger::new(),
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
                "params": [42]
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
            serde_json::from_value(json!({ "db": "primary", "sql": "SELECT n FROM t" })).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(q.row_count, 1);
        assert_eq!(q.rows[0]["n"], 42);
    }
}
