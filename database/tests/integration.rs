//! Integration: build a local AppState from a YAML config and exercise each
//! function handler end-to-end against an in-memory SQLite database.

use iii_database::config::WorkerConfig;
use iii_database::handle::HandleRegistry;
use iii_database::handlers::{execute, prepare, query, run_statement, transaction, AppState};
use iii_database::pool;
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
        json!({
            "db": "primary",
            "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)"
        }),
    )
    .await
    .unwrap();

    // Insert via execute (multi-row VALUES is a single INSERT statement, OK for SQLite)
    let r = execute::handle(
        &st,
        json!({
            "db": "primary",
            "sql": "INSERT INTO t (n) VALUES (?), (?)",
            "params": [10, 20]
        }),
    )
    .await
    .unwrap();
    assert_eq!(r["affected_rows"], 2);

    // Read via query
    let r = query::handle(
        &st,
        json!({
            "db": "primary",
            "sql": "SELECT id, n FROM t ORDER BY id"
        }),
    )
    .await
    .unwrap();
    assert_eq!(r["row_count"], 2);

    // Prepare + run
    let p = prepare::handle(
        &st,
        json!({
            "db": "primary",
            "sql": "SELECT n FROM t WHERE id = ?"
        }),
    )
    .await
    .unwrap();
    let id = p["handle"]["id"].as_str().unwrap().to_string();
    let r = run_statement::handle(&st, json!({"handle_id": id, "params": [1]}))
        .await
        .unwrap();
    assert_eq!(r["row_count"], 1);

    // Transaction
    let r = transaction::handle(
        &st,
        json!({
            "db": "primary",
            "statements": [
                {"sql": "UPDATE t SET n = n + 1 WHERE id = ?", "params": [1]},
                {"sql": "UPDATE t SET n = n + 1 WHERE id = ?", "params": [2]},
            ]
        }),
    )
    .await
    .unwrap();
    assert_eq!(r["committed"], true);

    // Verify final state
    let r = query::handle(
        &st,
        json!({
            "db": "primary",
            "sql": "SELECT n FROM t ORDER BY id"
        }),
    )
    .await
    .unwrap();
    assert_eq!(r["rows"][0]["n"], 11);
    assert_eq!(r["rows"][1]["n"], 21);
}

#[test]
fn binary_name_matches_manifest() {
    assert_eq!(iii_database::worker_name(), "iii-database");
}
