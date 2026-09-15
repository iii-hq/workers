//! `database::execute` — write SQL.

use super::AppState;
use crate::driver;
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use crate::value::JsonParam;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, JsonSchema)]
pub struct ExecuteReq {
    /// Logical database name. Optional — omitting it targets the sole
    /// configured database, or `primary` when several are configured.
    #[serde(default)]
    pub db: Option<String>,
    /// The write statement. To get rows back — and to have them ride the
    /// `database::row-changed` event this write fires — put a `RETURNING`
    /// clause in the SQL itself: `INSERT INTO t (n) VALUES (?) RETURNING id`
    /// (SQLite, Postgres; MySQL has no RETURNING). Without one the response
    /// has `returned_rows: []` and the event carries only `affected_rows`.
    #[serde(alias = "query")]
    pub sql: String,
    #[serde(default, deserialize_with = "crate::handlers::lenient_params")]
    pub params: Vec<Value>,
    /// Optional, and NOT what produces rows: this never adds a `RETURNING`
    /// clause or projects columns — write the clause into `sql`. SQLite
    /// refuses a non-empty list when the statement returns no rows
    /// (`RETURNING_MISMATCH`), so a write cannot silently lose its identity;
    /// Postgres and MySQL ignore the list with a warning.
    #[serde(default)]
    pub returning: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecuteResp {
    pub affected_rows: u64,
    /// SQLite/MySQL: the engine's last insert id, INSERT only. Postgres has
    /// none, so it is the first column of the first `RETURNING` row of an
    /// INSERT — put the key first: `RETURNING id, name`.
    pub last_insert_id: Option<String>,
    /// The rows the SQL's `RETURNING` clause produced (or a `SELECT`/`VALUES`
    /// sent here). Empty without such a clause.
    pub returned_rows: Vec<serde_json::Map<String, Value>>,
}

pub async fn handle(state: &AppState, req: ExecuteReq) -> Result<ExecuteResp, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let pool = state.pool(&db).await.map_err(err_to_str)?;
    // Reject empty SQL uniformly. See the matching guard in query.rs for why
    // this is at the handler boundary rather than per-driver: postgres' driver
    // accepts empty SQL as a no-op success, sqlite/mysql reject — guarding
    // here keeps the worker's contract symmetric across all three.
    if req.sql.trim().is_empty() {
        return Err(err_to_str(crate::error::DbError::DriverError {
            driver: format!("{:?}", pool.driver()),
            code: None,
            message: "empty SQL".into(),
            failed_index: None,
        }));
    }
    crate::handlers::reject_tx_control_sql(&req.sql).map_err(err_to_str)?;
    let params = JsonParam::from_json_slice(&req.params).map_err(err_to_str)?;

    let result = match &pool {
        Pool::Sqlite(p) => driver::sqlite::execute(p, &req.sql, &params, &req.returning).await,
        Pool::Postgres(p) => driver::postgres::execute(p, &req.sql, &params, &req.returning).await,
        Pool::Mysql(p) => driver::mysql::execute(p, &req.sql, &params, &req.returning).await,
    }
    .map_err(err_to_str)?;

    let returned_rows =
        crate::handlers::query_rows_to_objects(&result.returned_columns, result.returned_rows);
    // The write is committed (autocommit) — announce it.
    state
        .emit_row_change(&db, &req.sql, result.affected_rows, Some(&returned_rows))
        .await;
    Ok(ExecuteResp {
        affected_rows: result.affected_rows,
        last_insert_id: result.last_insert_id,
        returned_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::handlers::AppState;
    use crate::pool::{Pool, SqlitePool};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state() -> AppState {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(RwLock::new(pools)),
            config: Arc::new(RwLock::new(crate::config::WorkerConfig::default())),
            handles: Arc::new(HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
        }
    }

    fn req(v: Value) -> ExecuteReq {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn request_accepts_query_alias_and_string_params() {
        let request = req(json!({
            "db": "primary",
            "query": "SELECT 1",
            "params": "[1,\"a\"]"
        }));

        assert_eq!(request.sql, "SELECT 1");
        assert_eq!(request.params, vec![json!(1), json!("a")]);
    }

    #[test]
    fn request_does_not_recursively_decode_string_items_inside_array() {
        let request = req(json!({
            "db": "primary",
            "sql": "SELECT 1",
            "params": ["[1]", "{\"a\":1}"]
        }));

        assert_eq!(request.params, vec![json!("[1]"), json!("{\"a\":1}")]);
    }

    #[test]
    fn request_rejects_invalid_params_forms() {
        let cases = [
            (json!(null), "params must be a JSON array"),
            (json!(""), "params string is not a JSON array"),
            (json!("   "), "params string is not a JSON array"),
            (json!("not json"), "params string is not a JSON array"),
            (json!("null"), "params string is not a JSON array"),
            (json!("{}"), "params string is not a JSON array"),
        ];

        for (params, expected) in cases {
            let error = serde_json::from_value::<ExecuteReq>(json!({
                "db": "primary",
                "sql": "SELECT 1",
                "params": params
            }))
            .err()
            .expect("invalid params should be rejected");
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?} in {error}"
            );
        }
    }

    #[test]
    fn request_rejects_sql_and_query_together() {
        let error = serde_json::from_value::<ExecuteReq>(json!({
            "db": "primary",
            "sql": "SELECT 1",
            "query": "SELECT 2"
        }))
        .err()
        .expect("duplicate SQL fields should be rejected");

        assert!(error.to_string().contains("duplicate field"));
    }

    #[test]
    fn request_schema_stays_canonical() {
        let schema = serde_json::to_value(schemars::schema_for!(ExecuteReq).schema).unwrap();
        let properties = schema["properties"]
            .as_object()
            .expect("request schema should have object properties");

        assert!(properties.contains_key("sql"));
        assert!(!properties.contains_key("query"));
        assert_eq!(properties["params"]["type"], "array");
    }

    /// The published schema is what an agent reads before calling (through
    /// `engine::functions::info` and iii-directory): it has to say that rows
    /// come from a RETURNING clause in the SQL, and that the `returning`
    /// option is neither required nor what produces them.
    #[test]
    fn request_schema_explains_where_returned_rows_come_from() {
        let schema = serde_json::to_value(schemars::schema_for!(ExecuteReq).schema).unwrap();
        let properties = &schema["properties"];
        let sql_doc = properties["sql"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(sql_doc.contains("RETURNING"), "{sql_doc}");
        assert!(sql_doc.contains("row-changed"), "{sql_doc}");
        let returning_doc = properties["returning"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(returning_doc.contains("never adds"), "{returning_doc}");
        assert_eq!(properties["returning"]["type"], "array");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(required.contains(&"sql"));
        assert!(!required.contains(&"returning"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_rejects_transaction_control_sql_with_pointer() {
        // rctest5 postmortem: agents run execute("BEGIN") + execute("COMMIT")
        // expecting a session — but each call draws a fresh pooled
        // connection, and the leaked BEGIN poisoned the pool for every later
        // caller. The guard must name the real transactional surfaces.
        let st = state();
        for sql in ["BEGIN", "commit;", "  ROLLBACK", "/* tx */ BEGIN IMMEDIATE"] {
            let err = handle(&st, req(json!({"db":"primary","sql": sql})))
                .await
                .unwrap_err();
            assert!(err.contains("INVALID_PARAM"), "{sql}: {err}");
            assert!(err.contains("beginTransaction"), "{sql}: {err}");
            assert!(err.contains("executeBatch"), "{sql}: {err}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_insert_returns_envelope() {
        let st = state();
        handle(
            &st,
            req(json!({
                "db": "primary",
                "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)"
            })),
        )
        .await
        .unwrap();

        let resp = handle(
            &st,
            req(json!({
                "db": "primary",
                "sql": "INSERT INTO t (n) VALUES (?)",
                "params": [42]
            })),
        )
        .await
        .unwrap();
        assert_eq!(resp.affected_rows, 1);
        assert_eq!(resp.last_insert_id.as_deref(), Some("1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_update_with_no_prior_insert_returns_null_last_insert_id() {
        // SQLite's `last_insert_rowid()` is sticky per-connection — it stays
        // set across non-INSERT statements until another INSERT runs. To
        // exercise the None branch we run an UPDATE against a freshly-created
        // table without any prior INSERT on this pool's connection.
        let st = state();
        handle(
            &st,
            req(json!({
                "db": "primary",
                "sql": "CREATE TABLE t (n INT)"
            })),
        )
        .await
        .unwrap();
        let resp = handle(
            &st,
            req(json!({
                "db": "primary",
                "sql": "UPDATE t SET n = ? WHERE n = ?",
                "params": [99, 1]
            })),
        )
        .await
        .unwrap();
        assert_eq!(resp.affected_rows, 0);
        // No INSERT has ever run on this connection, so last_insert_rowid()
        // is 0 → driver returns None → JSON null (NOT the empty string "").
        assert!(
            resp.last_insert_id.is_none(),
            "expected None, got {:?}",
            resp.last_insert_id
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_update_after_insert_does_not_carry_stale_last_insert_id() {
        // Regression: SQLite's last_insert_rowid() is sticky per-connection,
        // and the pool reuses connections. Without an is_insert() guard, an
        // UPDATE running on a connection whose prior caller ran an INSERT
        // would report the prior INSERT's rowid as last_insert_id — a phantom
        // success signal that corrupts caller logic.
        let st = state();
        handle(
            &st,
            req(json!({"db":"primary","sql":"CREATE TABLE t (id INTEGER PRIMARY KEY, n INT)"})),
        )
        .await
        .unwrap();
        let ins = handle(
            &st,
            req(json!({"db":"primary","sql":"INSERT INTO t (n) VALUES (?)","params":[1]})),
        )
        .await
        .unwrap();
        assert_eq!(ins.last_insert_id.as_deref(), Some("1"));
        // Same pool, same connection (default max). The UPDATE must NOT
        // surface the rowid the INSERT just set.
        let upd = handle(
            &st,
            req(json!({"db":"primary","sql":"UPDATE t SET n = ? WHERE id = ?","params":[2, 1]})),
        )
        .await
        .unwrap();
        assert_eq!(upd.affected_rows, 1);
        assert!(
            upd.last_insert_id.is_none(),
            "UPDATE response leaked stale rowid: {:?}",
            upd.last_insert_id
        );
    }
}
