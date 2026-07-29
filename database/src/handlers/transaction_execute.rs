//! `database::transactionExecute` — write SQL inside an interactive
//! transaction. Same response envelope as `database::execute`. Bare
//! `BEGIN` / `COMMIT` / `ROLLBACK` / `END` / `SAVEPOINT` / `RELEASE` /
//! `SET TRANSACTION` / `START TRANSACTION` statements are rejected with
//! `INVALID_PARAM` so callers cannot side-channel finalization through SQL —
//! they must use the dedicated `commitTransaction` / `rollbackTransaction`
//! handlers. The detection runs *after* stripping leading SQL comments and
//! `;`, so `/* */COMMIT` and `--\nCOMMIT` are rejected too. See
//! [`super::tx_sql_guard`] for the full coverage and rationale.

use super::tx_sql_guard::is_transaction_control_sql;
use super::AppState;
use crate::driver;
use crate::error::DbError;
use crate::handle::PinnedConn;
use crate::handlers::query::err_to_str;
use crate::transaction::driver_system;
use crate::value::JsonParam;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, JsonSchema)]
pub struct TxExecuteReq {
    pub transaction_id: String,
    #[serde(alias = "query")]
    pub sql: String,
    #[serde(default, deserialize_with = "crate::handlers::lenient_params")]
    pub params: Vec<Value>,
    #[serde(default)]
    pub returning: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TxExecuteResp {
    pub affected_rows: u64,
    pub last_insert_id: Option<String>,
    pub returned_rows: Vec<serde_json::Map<String, Value>>,
}

pub async fn handle(state: &AppState, req: TxExecuteReq) -> Result<TxExecuteResp, String> {
    if req.sql.trim().is_empty() {
        return Err(err_to_str(DbError::DriverError {
            driver: "(tx)".into(),
            code: None,
            message: "empty SQL".into(),
            failed_index: None,
        }));
    }
    if is_transaction_control_sql(&req.sql) {
        // Reject before lookup so we don't consume the per-tx mutex on a
        // request that's just going to bounce. The caller gets a clear
        // pointer at the right handler.
        state.log.warn(
            "db_tx_finalization_via_sql_rejected",
            Some(json!({
                "db.transaction.id": req.transaction_id,
                "db.statement": req.sql.trim(),
            })),
        );
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "transaction-control SQL (BEGIN/START TRANSACTION/COMMIT/ROLLBACK/\
                    SAVEPOINT/RELEASE/END/SET TRANSACTION, including comment-prefixed forms) \
                    is not allowed in transactionExecute; use commitTransaction or \
                    rollbackTransaction"
                .into(),
        }));
    }
    let params = JsonParam::from_json_slice(&req.params).map_err(err_to_str)?;

    let lock = match state.transactions.lock(&req.transaction_id).await {
        Ok(l) => l,
        Err(e) => {
            state.log.warn(
                "db_tx_unknown",
                Some(json!({
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "EXECUTE",
                })),
            );
            return Err(err_to_str(e));
        }
    };

    let crate::transaction::TxLock {
        db_name,
        driver,
        guard: mut g,
        ..
    } = lock;

    let result = match &mut *g {
        PinnedConn::Sqlite(slot) => {
            driver::sqlite::tx_execute(slot, &req.sql, &params, &req.returning).await
        }
        PinnedConn::Postgres(client) => {
            driver::postgres::tx_execute(client, &req.sql, &params, &req.returning).await
        }
        PinnedConn::Mysql(conn) => {
            driver::mysql::tx_execute(conn, &req.sql, &params, &req.returning).await
        }
    };

    let response = match result {
        Ok(er) => {
            state.log.debug(
                "db_tx_statement",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "EXECUTE",
                    "db.statement": req.sql.trim(),
                    "affected_rows": er.affected_rows,
                })),
            );
            let returned_rows =
                crate::handlers::query_rows_to_objects(&er.returned_columns, er.returned_rows);
            // Staged, not announced: an interactive transaction's write is
            // invisible to everyone else until COMMIT, and announcing it here
            // would advertise rows a ROLLBACK is about to erase.
            state.stage_row_change(
                &req.transaction_id,
                &db_name,
                &req.sql,
                er.affected_rows,
                Some(&returned_rows),
            )
            .await;
            Ok(TxExecuteResp {
                affected_rows: er.affected_rows,
                last_insert_id: er.last_insert_id,
                returned_rows,
            })
        }
        Err(e) => {
            state.log.warn(
                "db_tx_statement_failed",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "EXECUTE",
                    "db.statement": req.sql.trim(),
                    "error": serde_json::to_value(&e).ok(),
                })),
            );
            Err(err_to_str(e))
        }
    };
    // Finalizers synchronize on this guard; keep it through event staging so
    // commit/rollback cannot overtake the statement between SQL and bookkeeping.
    drop(g);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::begin_transaction::tests::state;
    use serde_json::{json, Value};

    fn req(v: Value) -> TxExecuteReq {
        serde_json::from_value(v).unwrap()
    }

    // Keyword-set + comment-strip unit coverage lives in
    // `super::tx_sql_guard`. The tests below assert the handler's
    // *behaviour* — that the rejection actually fires before the registry
    // is touched, the log event lands, and the error message points at the
    // right finalization handler. We exercise enough bypass shapes here to
    // catch any future regression that re-introduces a side-channel.

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_commit_via_execute() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let err = handle(
            &st,
            req(json!({ "transaction_id": begin.transaction.id, "sql": "COMMIT" })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("commitTransaction"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_rollback_via_execute() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let err = handle(
            &st,
            req(json!({ "transaction_id": begin.transaction.id, "sql": "ROLLBACK" })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("rollbackTransaction"), "got: {err}");
    }

    /// Regression: `/* ... */COMMIT` previously fell through the first-token
    /// check (the first whitespace-delimited token was `/*`). After the
    /// strip-leading-noise fix it must reject like a bare `COMMIT`.
    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_block_comment_prefixed_commit() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let err = handle(
            &st,
            req(json!({
                "transaction_id": begin.transaction.id,
                "sql": "/* sneak */COMMIT",
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("commitTransaction"), "got: {err}");
    }

    /// Regression: `--text\nCOMMIT` previously bypassed the filter the same
    /// way as the block-comment form.
    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_line_comment_prefixed_commit() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let err = handle(
            &st,
            req(json!({
                "transaction_id": begin.transaction.id,
                "sql": "-- sneak\nCOMMIT",
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
    }

    /// Regression: `START TRANSACTION` (MySQL/ANSI spelling of `BEGIN`) was
    /// not in the keyword set; on MySQL it implicitly committed the outer
    /// txn and opened a new untracked one. Must now reject.
    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_start_transaction_via_execute() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let err = handle(
            &st,
            req(json!({
                "transaction_id": begin.transaction.id,
                "sql": "START TRANSACTION",
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
    }

    /// Success-path INSERT inside an interactive transaction surfaces
    /// `affected_rows` and `last_insert_id` just like the standalone
    /// `database::execute`. Uses an on-disk sqlite so the txn's pinned
    /// conn and the post-commit verification share one database file.
    #[tokio::test(flavor = "multi_thread")]
    async fn execute_insert_inside_tx_returns_affected_rows_and_last_insert_id() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let url = format!("sqlite:{}", tmp.path().display());
        let pool =
            crate::pool::SqlitePool::new(&url, &crate::config::PoolConfig::default()).unwrap();
        let mut pools = std::collections::HashMap::new();
        pools.insert("primary".to_string(), crate::pool::Pool::Sqlite(pool));
        let st = crate::handlers::AppState {
            pools: std::sync::Arc::new(tokio::sync::RwLock::new(pools)),
            config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::config::WorkerConfig::default(),
            )),
            handles: std::sync::Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
        };

        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(
                json!({ "db": "primary", "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)" }),
            )
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

        let resp = handle(
            &st,
            req(json!({
                "transaction_id": begin.transaction.id,
                "sql": "INSERT INTO t (n) VALUES (?)",
                "params": [7]
            })),
        )
        .await
        .unwrap();
        assert_eq!(resp.affected_rows, 1);
        assert_eq!(resp.last_insert_id.as_deref(), Some("1"));
    }
}
