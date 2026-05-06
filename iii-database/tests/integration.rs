//! Integration: build a local AppState from a YAML config and exercise each
//! function handler end-to-end against an in-memory SQLite database.

use iii_database::config::WorkerConfig;
use iii_database::handle::HandleRegistry;
use iii_database::handlers::execute::ExecuteReq;
use iii_database::handlers::prepare::PrepareReq;
use iii_database::handlers::query::QueryReq;
use iii_database::handlers::run_statement::RunReq;
use iii_database::handlers::transaction::TxReq;
use iii_database::handlers::{execute, prepare, query, run_statement, transaction, AppState};
use iii_database::pool;
use iii_sdk::RegisterFunction;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

async fn build_state() -> AppState {
    let yaml = "databases:\n  primary:\n    url: \"sqlite::memory:\"\n";
    let cfg = WorkerConfig::from_yaml(yaml).unwrap();
    let mut pools = HashMap::new();
    for (name, db) in &cfg.databases {
        let p = pool::build(name, db).await.unwrap();
        pools.insert(name.clone(), p);
    }
    AppState {
        pools: Arc::new(pools),
        handles: Arc::new(HandleRegistry::new()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn end_to_end_query_execute_prepare_run_transaction() {
    let st = build_state().await;

    // Schema setup via execute
    execute::handle(
        &st,
        serde_json::from_value::<ExecuteReq>(json!({
            "db": "primary",
            "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)"
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    // Insert via execute (multi-row VALUES is a single INSERT statement, OK for SQLite)
    let r = execute::handle(
        &st,
        serde_json::from_value::<ExecuteReq>(json!({
            "db": "primary",
            "sql": "INSERT INTO t (n) VALUES (?), (?)",
            "params": [10, 20]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.affected_rows, 2);

    // Read via query
    let r = query::handle(
        &st,
        serde_json::from_value::<QueryReq>(json!({
            "db": "primary",
            "sql": "SELECT id, n FROM t ORDER BY id"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.row_count, 2);

    // Prepare + run
    let p = prepare::handle(
        &st,
        serde_json::from_value::<PrepareReq>(json!({
            "db": "primary",
            "sql": "SELECT n FROM t WHERE id = ?"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    let id = p.handle.id.clone();
    let r = run_statement::handle(
        &st,
        serde_json::from_value::<RunReq>(json!({"handle_id": id, "params": [1]})).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.row_count, 1);

    // Transaction
    let r = transaction::handle(
        &st,
        serde_json::from_value::<TxReq>(json!({
            "db": "primary",
            "statements": [
                {"sql": "UPDATE t SET n = n + 1 WHERE id = ?", "params": [1]},
                {"sql": "UPDATE t SET n = n + 1 WHERE id = ?", "params": [2]},
            ]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(r.committed);

    // Verify final state
    let r = query::handle(
        &st,
        serde_json::from_value::<QueryReq>(json!({
            "db": "primary",
            "sql": "SELECT n FROM t ORDER BY id"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.rows[0]["n"], 11);
    assert_eq!(r.rows[1]["n"], 21);
}

#[test]
fn binary_name_matches_manifest() {
    assert_eq!(iii_database::worker_name(), "iii-database");
}

/// Regression: every RPC function must register through the typed
/// `RegisterFunction::new_async` API so the engine receives auto-generated
/// JSON Schemas. Without this the public API Reference shows empty schemas.
/// If someone adds a new function via `register_function_with(...)`, this test
/// won't catch it directly — but it locks the typed shape for the existing 5.
#[test]
fn registered_functions_carry_request_and_response_schemas() {
    fn assert_schemas<T, F, Fut, R, E>(id: &str, f: F)
    where
        T: serde::de::DeserializeOwned + schemars::JsonSchema + Send + 'static,
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<R, E>> + Send + 'static,
        R: serde::Serialize + schemars::JsonSchema + Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        let reg = RegisterFunction::new_async(id, f);
        assert!(
            reg.request_format().is_some(),
            "{id} missing request_format — did you switch back to register_function_with?"
        );
        assert!(
            reg.response_format().is_some(),
            "{id} missing response_format"
        );
    }

    // We can't move a real AppState into these closures (it owns DB pools),
    // so we just verify the schema-derivation path with the public Req/Resp
    // types. Any drift in the typed contract surfaces here as a compile error.
    async fn _q(_: QueryReq) -> Result<query::QueryResp, String> {
        unreachable!()
    }
    async fn _e(_: ExecuteReq) -> Result<execute::ExecuteResp, String> {
        unreachable!()
    }
    async fn _p(_: PrepareReq) -> Result<prepare::PrepareResp, String> {
        unreachable!()
    }
    async fn _r(_: RunReq) -> Result<query::QueryResp, String> {
        unreachable!()
    }
    async fn _t(_: TxReq) -> Result<transaction::TxResp, String> {
        unreachable!()
    }

    assert_schemas("iii-database::query", _q);
    assert_schemas("iii-database::execute", _e);
    assert_schemas("iii-database::prepareStatement", _p);
    assert_schemas("iii-database::runStatement", _r);
    assert_schemas("iii-database::transaction", _t);
}
