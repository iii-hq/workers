//! `iii-database::transactionQuery` — read SQL inside an interactive
//! transaction. Same response envelope as `iii-database::query`; the only
//! difference is the SQL runs on the pinned transaction connection rather
//! than a freshly-pooled one.

use super::AppState;
use crate::driver;
use crate::handle::PinnedConn;
use crate::handlers::query::QueryResp;
use crate::handlers::{query::err_to_str, query_rows_to_objects};
use crate::transaction::driver_system;
use crate::value::JsonParam;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize, JsonSchema)]
pub struct TxQueryReq {
    pub transaction_id: String,
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Value>,
}

pub async fn handle(state: &AppState, req: TxQueryReq) -> Result<QueryResp, String> {
    if req.sql.trim().is_empty() {
        return Err(err_to_str(crate::error::DbError::DriverError {
            driver: "(tx)".into(),
            code: None,
            message: "empty SQL".into(),
            failed_index: None,
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
                    "db.operation": "QUERY",
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
        PinnedConn::Sqlite(slot) => driver::sqlite::run_prepared(slot, &req.sql, &params).await,
        PinnedConn::Postgres(client) => {
            driver::postgres::run_prepared(client, &req.sql, &params).await
        }
        PinnedConn::Mysql(conn) => driver::mysql::run_prepared(conn, &req.sql, &params).await,
    };

    match result {
        Ok(qr) => {
            state.log.debug(
                "db_tx_statement",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "QUERY",
                    "db.statement": req.sql.trim(),
                    "row_count": qr.rows.len(),
                })),
            );
            let row_count = qr.rows.len();
            let rows = query_rows_to_objects(&qr.columns, qr.rows);
            Ok(QueryResp {
                rows,
                row_count,
                columns: qr.columns,
            })
        }
        Err(e) => {
            state.log.warn(
                "db_tx_statement_failed",
                Some(json!({
                    "db.system": driver_system(driver),
                    "db.name": db_name,
                    "db.transaction.id": req.transaction_id,
                    "db.operation": "QUERY",
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

    fn req(v: Value) -> TxQueryReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_inside_active_tx_returns_rows() {
        let st = state();
        // Open a transaction
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let tx_id = begin.transaction.id;

        let resp = handle(
            &st,
            req(json!({
                "transaction_id": tx_id,
                "sql": "SELECT 1 AS n"
            })),
        )
        .await
        .unwrap();
        assert_eq!(resp.row_count, 1);
        assert_eq!(resp.rows[0]["n"], 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_transaction_id_returns_not_found() {
        let st = state();
        let err = handle(
            &st,
            req(json!({
                "transaction_id": "00000000-0000-0000-0000-000000000000",
                "sql": "SELECT 1"
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("TRANSACTION_NOT_FOUND"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_sql_rejected_at_handler_boundary() {
        let st = state();
        let begin = crate::handlers::begin_transaction::handle(
            &st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        let err = handle(
            &st,
            req(json!({ "transaction_id": begin.transaction.id, "sql": "  " })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("empty SQL"), "got: {err}");
    }
}
