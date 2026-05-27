//! Integration: build a local AppState from a YAML config and exercise each
//! function handler end-to-end against an in-memory SQLite database.

use database::config::WorkerConfig;
use database::handle::HandleRegistry;
use database::handlers::execute::ExecuteReq;
use database::handlers::prepare::PrepareReq;
use database::handlers::query::QueryReq;
use database::handlers::run_statement::RunReq;
use database::handlers::transaction::TxReq;
use database::handlers::{execute, prepare, query, run_statement, transaction, AppState};
use database::pool;
use database::transaction::TxRegistry;
use iii_observability::Logger;
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
        transactions: TxRegistry::new(),
        log: Logger::new(),
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
    assert_eq!(database::worker_name(), "database");
}
