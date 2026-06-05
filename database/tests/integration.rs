//! Integration: build a local AppState from a YAML config and exercise each
//! function handler end-to-end against an in-memory SQLite database.

use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::execute::ExecuteReq;
use database::handlers::list_databases::{self, ListDatabasesReq};
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
use tokio::sync::RwLock;

async fn build_state() -> AppState {
    let yaml = "databases:\n  primary:\n    url: \"sqlite::memory:\"\n";
    let cfg = WorkerConfig::from_yaml(yaml).unwrap();
    let mut pools = HashMap::new();
    for (name, db) in &cfg.databases {
        let p = pool::build(name, db).await.unwrap();
        pools.insert(name.clone(), p);
    }
    AppState {
        pools: Arc::new(RwLock::new(pools)),
        config: Arc::new(RwLock::new(cfg)),
        handles: Arc::new(HandleRegistry::new()),
        transactions: TxRegistry::new(),
        log: Logger::new(),
    }
}

#[test]
fn from_json_parity_with_yaml_seed_shape() {
    let yaml = "databases:\n  primary:\n    url: \"sqlite::memory:\"\n";
    let from_yaml = WorkerConfig::from_yaml(yaml).unwrap();
    let from_json = WorkerConfig::from_json(&from_yaml.to_json()).unwrap();
    assert_eq!(
        from_yaml.databases["primary"].url,
        from_json.databases["primary"].url
    );
    assert_eq!(
        from_yaml.databases["primary"].driver,
        from_json.databases["primary"].driver
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn list_databases_reports_configured_primary() {
    // Arrange
    let st = build_state().await;

    // Act
    let resp = list_databases::handle(&st, ListDatabasesReq::default())
        .await
        .unwrap();

    // Assert
    assert_eq!(resp.count, 1);
    assert_eq!(resp.databases[0].name, "primary");
    assert_eq!(resp.databases[0].driver, "sqlite");
    assert_eq!(resp.databases[0].url, "sqlite::memory:");
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_config_updates_list_snapshot() {
    // Arrange
    let st = build_state().await;
    let new_yaml =
        "databases:\n  first:\n    url: \"sqlite::memory:\"\n  second:\n    url: \"sqlite::memory:\"\n";
    let new_cfg = WorkerConfig::from_yaml(new_yaml).unwrap();

    // Act
    configuration::apply_config(&st, new_cfg).await.unwrap();
    let resp = list_databases::handle(&st, ListDatabasesReq::default())
        .await
        .unwrap();

    // Assert
    assert_eq!(resp.count, 2);
    let names: Vec<&str> = resp.databases.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn binary_name_matches_manifest() {
    assert_eq!(database::worker_name(), "database");
}

/// Regression for the registry-publish crash (`unable to open database file:
/// ./data/iii.db`). This drives the *startup* dispatch path the worker uses at
/// boot — `pool::build` (called by `main.rs` via `configuration::build_pools`)
/// — for a file-backed sqlite database whose parent directory does not exist
/// yet, mirroring the default `sqlite:./data/iii.db` config running from a
/// clean checkout. The pool must build (creating the parent dir) and serve a
/// connection rather than dying on startup.
#[tokio::test(flavor = "multi_thread")]
async fn build_pool_creates_missing_sqlite_parent_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("data").join("iii.db");
    assert!(!db_path.parent().unwrap().exists());

    let url = format!("sqlite:{}", db_path.display());
    let yaml = format!("databases:\n  primary:\n    url: \"{url}\"\n");
    let cfg = WorkerConfig::from_yaml(&yaml).unwrap();

    let mut pools = HashMap::new();
    for (name, db) in &cfg.databases {
        let p = pool::build(name, db)
            .await
            .expect("startup pool build must create the missing parent dir");
        pools.insert(name.clone(), p);
    }
    assert!(db_path.parent().unwrap().exists());

    let st = AppState {
        pools: Arc::new(RwLock::new(pools)),
        config: Arc::new(RwLock::new(cfg)),
        handles: Arc::new(HandleRegistry::new()),
        transactions: TxRegistry::new(),
        log: Logger::new(),
    };

    // The freshly-created on-disk db is usable end-to-end.
    execute::handle(
        &st,
        serde_json::from_value::<ExecuteReq>(json!({
            "db": "primary",
            "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY)"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    let r = query::handle(
        &st,
        serde_json::from_value::<QueryReq>(json!({
            "db": "primary",
            "sql": "SELECT count(*) AS n FROM t"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.row_count, 1);
}
