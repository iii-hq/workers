//! `iii-database::runStatement` — run a previously-prepared handle.

use super::AppState;
use crate::driver;
use crate::handle::PinnedConn;
use crate::handlers::query::QueryResp;
use crate::handlers::{query::err_to_str, query_rows_to_objects};
use crate::value::JsonParam;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, JsonSchema)]
pub struct RunReq {
    pub handle_id: String,
    #[serde(default)]
    pub params: Vec<Value>,
}

pub async fn handle(state: &AppState, req: RunReq) -> Result<QueryResp, String> {
    let params = JsonParam::from_json_slice(&req.params).map_err(err_to_str)?;
    let (sql, mut guard) = state
        .handles
        .lock(&req.handle_id)
        .await
        .map_err(err_to_str)?;

    let result = match &mut *guard {
        PinnedConn::Sqlite(slot) => driver::sqlite::run_prepared(slot, &sql, &params).await,
        PinnedConn::Postgres(conn) => driver::postgres::run_prepared(conn, &sql, &params).await,
        PinnedConn::Mysql(conn) => driver::mysql::run_prepared(conn, &sql, &params).await,
    }
    .map_err(err_to_str)?;

    let row_count = result.rows.len();
    let rows = query_rows_to_objects(&result.columns, result.rows);
    Ok(QueryResp {
        rows,
        row_count,
        columns: result.columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::handlers::{prepare, AppState};
    use crate::pool::{Pool, SqlitePool};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn state_in_memory() -> AppState {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(pools),
            handles: Arc::new(HandleRegistry::new()),
        }
    }

    /// Build an AppState backed by a tempfile-backed SQLite DB.
    /// Returned `_tmp` keeps the file alive for the test duration.
    fn state_on_disk() -> (AppState, tempfile::NamedTempFile) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let url = format!("sqlite:{}", tmp.path().display());
        let pool = SqlitePool::new(&url, &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        let st = AppState {
            pools: Arc::new(pools),
            handles: Arc::new(HandleRegistry::new()),
        };
        (st, tmp)
    }

    fn run_req(v: Value) -> RunReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_unknown_handle_returns_statement_not_found() {
        let st = state_in_memory();
        let err = handle(
            &st,
            run_req(json!({
                "handle_id": "00000000-0000-0000-0000-000000000000",
                "params": []
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("STATEMENT_NOT_FOUND"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_then_run_returns_rows() {
        // Use a file-backed SQLite so that the `execute` setup conn and the
        // pinned `prepareStatement` conn see the same database.
        let (st, _tmp) = state_on_disk();
        // execute() runs a single statement at a time, so issue them separately.
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
        for n in [1, 2, 3] {
            crate::handlers::execute::handle(
                &st,
                serde_json::from_value(json!({
                    "db": "primary",
                    "sql": "INSERT INTO t (n) VALUES (?)",
                    "params": [n]
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        }

        let prep = prepare::handle(
            &st,
            serde_json::from_value(json!({
                "db": "primary",
                "sql": "SELECT n FROM t WHERE n > ? ORDER BY n"
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let id = prep.handle.id.clone();

        let resp = handle(
            &st,
            run_req(json!({
                "handle_id": id,
                "params": [1]
            })),
        )
        .await
        .unwrap();
        assert_eq!(resp.row_count, 2);
    }
}
