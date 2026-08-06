//! Integration: build a local AppState from a YAML config and exercise each
//! function handler end-to-end against an in-memory SQLite database.

use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::browse::{self, BrowseTableReq};
use database::handlers::catalog::{TableKind, TableRef};
use database::handlers::column_stats::{self, ColumnStatsReq};
use database::handlers::execute::ExecuteReq;
use database::handlers::explain::{self, ExplainReq};
use database::handlers::health::{self, HealthReq, TerminateReq};
use database::handlers::list_databases::{self, ListDatabasesReq};
use database::handlers::prepare::PrepareReq;
use database::handlers::query::QueryReq;
use database::handlers::run_statement::RunReq;
use database::handlers::schema::{self, DescribeSchemaReq, DescribeTableReq, ListTablesReq};
use database::handlers::transaction::TxReq;
use database::handlers::{execute, prepare, query, run_statement, transaction, AppState};
use database::pool;
use database::transaction::TxRegistry;
use iii_helpers::observability::Logger;
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
        row_changes: None,
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
    configuration::apply_config(&st, new_cfg, None)
        .await
        .unwrap();
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
        row_changes: None,
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

/* ---------------- catalog introspection ---------------- */

/// Schema with the shapes that break naive catalog readers: an implicit
/// foreign key (`REFERENCES users` with no column), a composite primary key,
/// a multi-column index, and a view.
async fn seed_catalog(st: &AppState) {
    for sql in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL, plan TEXT DEFAULT 'free')",
        // No target column: sqlite reports `to` as NULL and means "the parent's
        // primary key". A reader that trusts the NULL emits a broken reference.
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users, total REAL)",
        "CREATE TABLE order_items (order_id INTEGER, sku TEXT, qty INTEGER, \
         PRIMARY KEY (order_id, sku), FOREIGN KEY (order_id) REFERENCES orders(id))",
        "CREATE INDEX ix_orders_user_total ON orders (user_id, total)",
        "CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100",
    ] {
        execute::handle(
            st,
            serde_json::from_value::<ExecuteReq>(json!({"db": "primary", "sql": sql})).unwrap(),
        )
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_tables_reports_tables_and_views() {
    let st = build_state().await;
    seed_catalog(&st).await;

    let r = schema::list_tables(
        &st,
        serde_json::from_value::<ListTablesReq>(json!({"db": "primary"})).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(r.count, r.tables.len());
    let by_name: HashMap<&str, &TableRef> = r.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    assert!(by_name.contains_key("users"));
    assert!(matches!(by_name["orders"].kind, TableKind::Table));
    assert!(matches!(by_name["big_orders"].kind, TableKind::View));
    // sqlite has no namespace above the table.
    assert!(by_name["users"].schema.is_none());
    // Internal bookkeeping stays hidden.
    assert!(!r.tables.iter().any(|t| t.name.starts_with("sqlite_")));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_table_resolves_columns_keys_and_indexes() {
    let st = build_state().await;
    seed_catalog(&st).await;

    let d = schema::describe_table(
        &st,
        serde_json::from_value::<DescribeTableReq>(json!({"db": "primary", "table": "orders"}))
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(d.table, "orders");
    // Columns arrive in declaration order.
    let names: Vec<&str> = d.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["id", "user_id", "total"]);

    let id = &d.columns[0];
    assert!(id.primary_key);
    assert_eq!(id.position, 1);

    // The implicit `REFERENCES users` resolves to the parent's primary key
    // rather than surfacing an empty column.
    let user_id = &d.columns[1];
    let fk = user_id
        .foreign_key
        .as_ref()
        .expect("user_id is a foreign key");
    assert_eq!(fk.table, "users");
    assert_eq!(fk.column, "id");
    assert!(fk.schema.is_none());
    assert!(d.columns[2].foreign_key.is_none());

    // A multi-column index keeps its columns in ordinal order.
    let ix = d
        .indexes
        .iter()
        .find(|i| i.name == "ix_orders_user_total")
        .expect("index present");
    assert_eq!(ix.columns, vec!["user_id", "total"]);
    assert!(!ix.unique);
    assert!(!ix.primary);

    // sqlite has no cheap estimate, so it reports none rather than guessing.
    assert!(d.row_count_estimate.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_table_reports_composite_primary_keys() {
    let st = build_state().await;
    seed_catalog(&st).await;

    let d = schema::describe_table(
        &st,
        serde_json::from_value::<DescribeTableReq>(
            json!({"db": "primary", "table": "order_items"}),
        )
        .unwrap(),
    )
    .await
    .unwrap();

    let pk: Vec<&str> = d
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(pk, vec!["order_id", "sku"]);

    let fk = d.columns[0].foreign_key.as_ref().expect("explicit fk");
    assert_eq!((fk.table.as_str(), fk.column.as_str()), ("orders", "id"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_table_rejects_an_unknown_name() {
    let st = build_state().await;
    seed_catalog(&st).await;

    let err = schema::describe_table(
        &st,
        serde_json::from_value::<DescribeTableReq>(json!({"db": "primary", "table": "nope"}))
            .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("no such table: nope"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_schema_covers_every_table_in_one_pass() {
    let st = build_state().await;
    seed_catalog(&st).await;

    let all = schema::describe_schema(
        &st,
        serde_json::from_value::<DescribeSchemaReq>(
            json!({"db": "primary", "include_indexes": true}),
        )
        .unwrap(),
    )
    .await
    .unwrap();

    assert!(!all.truncated);
    assert_eq!(all.count, all.tables.len());
    let names: Vec<&str> = all.tables.iter().map(|t| t.table.as_str()).collect();
    for want in ["users", "orders", "order_items", "big_orders"] {
        assert!(names.contains(&want), "missing {want} in {names:?}");
    }

    // Relationships survive the batch path — this is what a diagram reads.
    let orders = all.tables.iter().find(|t| t.table == "orders").unwrap();
    assert_eq!(
        orders.columns[1]
            .foreign_key
            .as_ref()
            .map(|f| f.table.as_str()),
        Some("users")
    );

    // Restricting to a subset filters without changing the shape.
    let some = schema::describe_schema(
        &st,
        serde_json::from_value::<DescribeSchemaReq>(
            json!({"db": "primary", "tables": ["users", "orders"]}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(some.count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_schema_flags_truncation_instead_of_hiding_it() {
    let st = build_state().await;
    seed_catalog(&st).await;

    let capped = schema::describe_schema(
        &st,
        serde_json::from_value::<DescribeSchemaReq>(json!({"db": "primary", "max_tables": 2}))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(capped.count, 2);
    assert!(capped.truncated, "a cut-short result must say so");
}

/* ---------------- browseTable ---------------- */

async fn seed_rows(st: &AppState) {
    execute::handle(
        st,
        serde_json::from_value::<ExecuteReq>(json!({
            "db": "primary",
            "sql": "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, note TEXT, score INT)"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    for (name, note, score) in [
        ("ana", Some("50% off"), 10),
        ("bob", None, 20),
        ("cyd", Some("plain"), 30),
        ("dee", Some(""), 40),
        ("eve", Some("other"), 50),
    ] {
        execute::handle(
            st,
            serde_json::from_value::<ExecuteReq>(json!({
                "db": "primary",
                "sql": "INSERT INTO people (name, note, score) VALUES (?, ?, ?)",
                "params": [name, note, score]
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_pages_with_a_sentinel_and_counts_the_whole_table() {
    let st = build_state().await;
    seed_rows(&st).await;

    let first = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(
            json!({"db": "primary", "table": "people", "page_size": 2}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    // The sentinel row is fetched but never returned.
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    assert_eq!(first.total, Some(5));

    let last = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(
            json!({"db": "primary", "table": "people", "page": 2, "page_size": 2}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(last.rows.len(), 1);
    assert!(!last.has_more, "the final page has nothing after it");
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_total_reflects_the_filters_not_the_table() {
    let st = build_state().await;
    seed_rows(&st).await;

    let r = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people", "page_size": 2,
            "filters": [{"column": "score", "op": "gte", "value": 30}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    // A pager showing 5 while 3 rows match is the bug this prevents.
    assert_eq!(r.total, Some(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_treats_a_percent_in_a_filter_as_a_literal() {
    let st = build_state().await;
    seed_rows(&st).await;

    let r = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people",
            "filters": [{"column": "note", "op": "contains", "value": "50%"}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    // Unescaped, `%` would make this match every non-null note.
    assert_eq!(r.total, Some(1));
    assert_eq!(r.rows[0]["name"], "ana");
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_separates_null_from_the_empty_string() {
    let st = build_state().await;
    seed_rows(&st).await;

    let nulls = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people",
            "filters": [{"column": "note", "op": "is_null"}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(nulls.total, Some(1), "only bob has a NULL note");

    let empty = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people",
            "filters": [{"column": "note", "op": "is_empty"}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(empty.total, Some(2), "is_empty covers NULL and ''");
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_sorts_and_can_skip_the_count() {
    let st = build_state().await;
    seed_rows(&st).await;

    let r = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people",
            "sort": [{"column": "score", "direction": "desc"}],
            "include_total": false
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.rows[0]["name"], "eve");
    assert!(r.total.is_none(), "the count is skipped when not asked for");
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_follows_a_foreign_key_with_an_equality_filter() {
    let st = build_state().await;
    seed_rows(&st).await;

    // This is why there is no separate read-a-row function.
    let r = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people", "page_size": 1,
            "filters": [{"column": "id", "op": "equals", "value": 3}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0]["name"], "cyd");
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_refuses_an_incomplete_filter() {
    let st = build_state().await;
    seed_rows(&st).await;

    let err = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people",
            "filters": [{"column": "score", "op": "equals"}]
        }))
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("needs `value`"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_refuses_the_wrong_operand_instead_of_panicking() {
    // `value2` alone satisfied a count-based arity check and then unwrapped
    // the absent `value`. Reachable from the wire, so it aborted the task.
    let st = build_state().await;
    seed_rows(&st).await;

    let err = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({
            "db": "primary", "table": "people",
            "filters": [{"column": "score", "op": "equals", "value2": 5}]
        }))
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("needs `value`"), "got: {err}");
}

/* ---------------- explain ---------------- */

#[tokio::test(flavor = "multi_thread")]
async fn explain_returns_a_plan_tree_for_a_read() {
    let st = build_state().await;
    seed_rows(&st).await;

    let r = explain::handle(
        &st,
        serde_json::from_value::<ExplainReq>(json!({
            "db": "primary", "sql": "SELECT * FROM people WHERE score > 20"
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(r.format, explain::PlanFormat::SqliteQueryPlan);
    assert!(!r.analyzed, "sqlite has no ANALYZE form");
    let root = r.root.expect("a plan tree");
    assert!(!root.children.is_empty(), "the scan hangs off the root");
    assert_eq!(root.children[0].relation.as_deref(), Some("people"));
}

/// The gate that keeps a viewer from deleting data. `EXPLAIN ANALYZE DELETE`
/// really executes on postgres and mysql, so the worker refuses it outright
/// rather than trusting the caller to have checked.
#[tokio::test(flavor = "multi_thread")]
async fn explain_refuses_analyze_on_anything_that_writes() {
    let st = build_state().await;
    seed_rows(&st).await;

    for sql in [
        "DELETE FROM people",
        "UPDATE people SET score = 0",
        "DROP TABLE people",
        // Leads with WITH, but is a write.
        "WITH doomed AS (DELETE FROM people RETURNING *) SELECT * FROM doomed",
        // A second statement would go unchecked.
        "SELECT 1; DROP TABLE people",
        // A comment must not smuggle the verb past the check.
        "-- SELECT\nDELETE FROM people",
    ] {
        let err = explain::handle(
            &st,
            serde_json::from_value::<ExplainReq>(json!({
                "db": "primary", "sql": sql, "analyze": true
            }))
            .unwrap(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("read-only"),
            "should refuse `{sql}`, got: {err}"
        );
    }

    // The rows are all still there.
    let after = browse::handle(
        &st,
        serde_json::from_value::<BrowseTableReq>(json!({"db": "primary", "table": "people"}))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(after.total, Some(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn explain_allows_analyze_on_a_plain_read() {
    let st = build_state().await;
    seed_rows(&st).await;

    // Accepted by the gate; sqlite then reports it could not actually analyze.
    let r = explain::handle(
        &st,
        serde_json::from_value::<ExplainReq>(json!({
            "db": "primary", "sql": "SELECT * FROM people", "analyze": true
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(!r.analyzed);
}

/* ---------------- columnStats ---------------- */

#[tokio::test(flavor = "multi_thread")]
async fn column_stats_defaults_to_planner_statistics() {
    let st = build_state().await;
    seed_rows(&st).await;

    let r = column_stats::handle(
        &st,
        serde_json::from_value::<ColumnStatsReq>(json!({"db": "primary", "table": "people"}))
            .unwrap(),
    )
    .await
    .unwrap();

    // The default must never scan; it reports what the planner knows, clearly
    // labelled, even when that is very little.
    assert!(r.approximate);
    assert_eq!(r.columns.len(), 4);
    assert!(r
        .columns
        .iter()
        .all(|c| c.source == column_stats::StatSource::Planner));
    assert!(
        r.columns.iter().all(|c| c.top_values.is_empty()),
        "top values require a scan, so the cheap path does not invent them"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn column_stats_exact_counts_nulls_and_top_values() {
    let st = build_state().await;
    seed_rows(&st).await;

    let r = column_stats::handle(
        &st,
        serde_json::from_value::<ColumnStatsReq>(json!({
            "db": "primary", "table": "people", "columns": ["note"], "exact": true
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    assert!(!r.approximate);
    let note = &r.columns[0];
    assert_eq!(note.source, column_stats::StatSource::Computed);
    assert_eq!(note.row_count, Some(5));
    // bob's note is NULL; dee's is '' — counted as present, not null.
    assert_eq!(note.null_count, Some(1));
    assert_eq!(note.distinct_count, Some(4));
    assert_eq!(note.top_values.len(), 4);
    assert!(note.top_values.iter().all(|t| t.count == 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn column_stats_rejects_a_column_that_does_not_exist() {
    let st = build_state().await;
    seed_rows(&st).await;

    let err = column_stats::handle(
        &st,
        serde_json::from_value::<ColumnStatsReq>(json!({
            "db": "primary", "table": "people", "columns": ["nope"]
        }))
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("no such column `nope`"), "got: {err}");
}

/* ---------------- health ---------------- */

#[tokio::test(flavor = "multi_thread")]
async fn health_reports_pool_stats_and_says_what_sqlite_cannot_answer() {
    let st = build_state().await;
    seed_rows(&st).await;

    let h = health::handle(
        &st,
        serde_json::from_value::<HealthReq>(json!({"db": "primary"})).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(h.driver, "sqlite");
    assert_eq!(h.worker_version, env!("CARGO_PKG_VERSION"));
    // The pool always answers, on every driver.
    assert!(h.pool.max >= 1);
    assert!(h.pool.size.is_some(), "r2d2 exposes live counters");

    // The distinction that makes the report honest: sqlite has no sessions,
    // which is a different answer from "no queries are running".
    for section in [&h.active_queries] {
        match section {
            health::ProbeResult::Unsupported { reason } => {
                assert!(reason.contains("sqlite"), "got: {reason}")
            }
            other => panic!("expected unsupported, got {other:?}"),
        }
    }
    assert!(matches!(h.locks, health::ProbeResult::Unsupported { .. }));
    assert!(matches!(h.cache, health::ProbeResult::Unsupported { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn terminate_refuses_a_non_numeric_id_and_refuses_sqlite() {
    let st = build_state().await;

    let err = health::terminate(
        &st,
        serde_json::from_value::<TerminateReq>(json!({"db": "primary", "id": "1"})).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("in-process"), "got: {err}");

    // An id is interpolated into the statement, so it must parse as a number
    // before it gets anywhere near SQL.
    let err = health::terminate(
        &st,
        serde_json::from_value::<TerminateReq>(
            json!({"db": "primary", "id": "1); DROP TABLE people--"}),
        )
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("not a backend id"), "got: {err}");
}
