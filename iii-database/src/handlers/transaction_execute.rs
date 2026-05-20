//! `iii-database::transactionExecute` — write SQL inside an interactive
//! transaction. Same response envelope as `iii-database::execute`. Bare
//! `BEGIN` / `COMMIT` / `ROLLBACK` / `END` / `SAVEPOINT` / `RELEASE` /
//! `SET TRANSACTION` statements are rejected with `INVALID_PARAM` so
//! callers cannot side-channel finalization through SQL — they must use
//! the dedicated `commitTransaction` / `rollbackTransaction` handlers.

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
    pub sql: String,
    #[serde(default)]
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

/// First-token check for transaction-control SQL. We compare against the
/// uppercased trimmed first whitespace-delimited token (with any trailing
/// `;` stripped); this matches `BEGIN`, `COMMIT`, `ROLLBACK`, `END`,
/// `SAVEPOINT`, `RELEASE`, and `SET TRANSACTION ...` regardless of
/// surrounding whitespace and trailing arguments. Bare `SET` (not
/// `SET TRANSACTION`) is left alone — callers may legitimately set session
/// variables inside a transaction.
fn is_transaction_control_sql(sql: &str) -> bool {
    let trimmed = sql.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
    let mut tokens = trimmed.split_whitespace();
    let Some(first) = tokens.next() else {
        return false;
    };
    // `COMMIT;` / `end;` / `ROLLBACK ;` all reach here as the first token
    // including the trailing `;`. Strip it before matching so the same
    // keyword set covers terminated and unterminated forms.
    let first = first.trim_end_matches(';');
    let upper = first.to_ascii_uppercase();
    match upper.as_str() {
        "BEGIN" | "COMMIT" | "ROLLBACK" | "END" | "SAVEPOINT" | "RELEASE" => true,
        "SET" => tokens
            .next()
            .map(|t| t.eq_ignore_ascii_case("TRANSACTION"))
            .unwrap_or(false),
        _ => false,
    }
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
            reason: "transaction-control SQL (BEGIN/COMMIT/ROLLBACK/SAVEPOINT/SET TRANSACTION) \
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

    match result {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::begin_transaction::tests::state;
    use serde_json::{json, Value};

    fn req(v: Value) -> TxExecuteReq {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn is_transaction_control_sql_matches_finalization_keywords() {
        // Casing + leading whitespace + trailing args must all still match.
        assert!(is_transaction_control_sql("COMMIT"));
        assert!(is_transaction_control_sql(" rollback "));
        assert!(is_transaction_control_sql("ROLLBACK TO SAVEPOINT foo"));
        assert!(is_transaction_control_sql(
            "BEGIN ISOLATION LEVEL SERIALIZABLE"
        ));
        assert!(is_transaction_control_sql("end;"));
        assert!(is_transaction_control_sql("SAVEPOINT s1"));
        assert!(is_transaction_control_sql("RELEASE SAVEPOINT s1"));
        assert!(is_transaction_control_sql("SET TRANSACTION READ ONLY"));
        assert!(is_transaction_control_sql(
            "set transaction isolation level serializable"
        ));
    }

    #[test]
    fn is_transaction_control_sql_passes_normal_dml() {
        // Bare SET (e.g. SET LOCAL search_path) is fine.
        assert!(!is_transaction_control_sql("INSERT INTO t (n) VALUES (1)"));
        assert!(!is_transaction_control_sql("UPDATE t SET n = 2"));
        assert!(!is_transaction_control_sql("DELETE FROM t"));
        assert!(!is_transaction_control_sql("SELECT 1"));
        assert!(!is_transaction_control_sql("SET LOCAL search_path = foo"));
    }

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

    /// Success-path INSERT inside an interactive transaction surfaces
    /// `affected_rows` and `last_insert_id` just like the standalone
    /// `iii-database::execute`. Uses an on-disk sqlite so the txn's pinned
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
            pools: std::sync::Arc::new(pools),
            handles: std::sync::Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_sdk::Logger::new(),
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
