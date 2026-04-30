//! SQLite driver methods. Each function takes the pool, runs work via
//! `spawn_blocking`, and returns the shared driver types.

use crate::driver::{
    ColumnMeta, ExecuteResult, Isolation, QueryResult, Row, TxStatement, TxStepResult,
};
use crate::error::DbError;
use crate::pool::SqlitePool;
use crate::value::{JsonParam, RowValue};
use rusqlite::types::{Value as SqlValue, ValueRef};

pub async fn query(
    pool: &SqlitePool,
    sql: &str,
    params: &[JsonParam],
    _timeout_ms: u64, // SQLite has no per-query timeout; honored via spawn_blocking budget upstream
) -> Result<QueryResult, DbError> {
    let conn = pool.acquire().await?;
    let sql = sql.to_string();
    let params = params.to_vec();

    tokio::task::spawn_blocking(move || -> Result<QueryResult, DbError> {
        conn.with(|c| {
            let mut stmt = c.prepare(&sql).map_err(map_err)?;
            let columns: Vec<ColumnMeta> = stmt
                .columns()
                .into_iter()
                .map(|col| ColumnMeta {
                    name: col.name().to_string(),
                    ty: col.decl_type().unwrap_or("").to_string(),
                })
                .collect();

            let bound: Vec<SqlValue> = params.iter().map(json_param_to_sql).collect();
            let bound_refs: Vec<&dyn rusqlite::ToSql> =
                bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

            let n = columns.len();
            let mut rows_out: Vec<Row> = Vec::new();
            let mut rows = stmt.query(bound_refs.as_slice()).map_err(map_err)?;
            while let Some(row) = rows.next().map_err(map_err)? {
                let mut vals = Vec::with_capacity(n);
                for i in 0..n {
                    vals.push(row_value_at(row, i)?);
                }
                rows_out.push(Row(vals));
            }
            Ok(QueryResult {
                columns,
                rows: rows_out,
            })
        })
    })
    .await
    .map_err(|e| DbError::DriverError {
        driver: "sqlite".into(),
        code: None,
        message: format!("spawn_blocking join: {e}"),
        failed_index: None,
    })?
}

fn json_param_to_sql(p: &JsonParam) -> SqlValue {
    match p {
        JsonParam::Null => SqlValue::Null,
        JsonParam::Bool(b) => SqlValue::Integer(if *b { 1 } else { 0 }),
        JsonParam::Int(i) => SqlValue::Integer(*i),
        JsonParam::Float(f) => SqlValue::Real(*f),
        JsonParam::Text(s) => SqlValue::Text(s.clone()),
        JsonParam::Json(v) => SqlValue::Text(v.to_string()),
    }
}

fn row_value_at(row: &rusqlite::Row<'_>, idx: usize) -> Result<RowValue, DbError> {
    let r: ValueRef = row.get_ref(idx).map_err(map_err)?;
    Ok(match r {
        ValueRef::Null => RowValue::Null,
        ValueRef::Integer(i) => RowValue::Int(i),
        ValueRef::Real(f) => RowValue::Float(f),
        ValueRef::Text(t) => RowValue::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => RowValue::Bytes(b.to_vec()),
    })
}

pub(crate) fn map_err(e: rusqlite::Error) -> DbError {
    let code = match &e {
        rusqlite::Error::SqliteFailure(f, _) => Some(format!("{:?}", f.code)),
        _ => None,
    };
    DbError::DriverError {
        driver: "sqlite".into(),
        code,
        message: e.to_string(),
        failed_index: None,
    }
}

/// Pessimistic multi-statement detector. After stripping trailing
/// whitespace and semicolons, any remaining `;` is treated as a separator.
/// String-literal edge cases (e.g. a `;` inside a quoted string) are not
/// handled — for v1.0, false positives are an acceptable price for
/// preventing silent statement-drop in `Connection::execute`.
fn looks_like_multi_statement(sql: &str) -> bool {
    let trimmed = sql.trim_end_matches(|c: char| c.is_whitespace() || c == ';');
    trimmed.contains(';')
}

/// True when the SQL statement is an INSERT. Used to gate `last_insert_rowid()`
/// reporting: that function is sticky per-connection and pool reuse means a
/// non-INSERT statement on a connection that previously inserted will still
/// see the prior rowid.
///
/// Naïve prefix check by design: false-negatives (e.g. `REPLACE INTO …` or
/// `WITH cte AS (…) INSERT …`) fall through to `last_insert_id: None`, which
/// is safe — the alternative is leaking a stale rowid from a prior pool
/// caller's INSERT, which is what we're guarding against.
fn is_insert(sql: &str) -> bool {
    sql.trim_start()
        .to_ascii_uppercase()
        .starts_with("INSERT")
}

pub async fn execute(
    pool: &SqlitePool,
    sql: &str,
    params: &[JsonParam],
    returning: &[String],
) -> Result<ExecuteResult, DbError> {
    if looks_like_multi_statement(sql) {
        return Err(DbError::DriverError {
            driver: "sqlite".into(),
            code: Some("MULTI_STATEMENT".into()),
            message: "rusqlite execute() supports only a single statement; \
                      use multiple execute() calls or execute_batch via DDL"
                .into(),
            failed_index: None,
        });
    }
    let conn = pool.acquire().await?;
    let sql = sql.to_string();
    let params = params.to_vec();
    let has_returning = !returning.is_empty() || sql.to_ascii_uppercase().contains(" RETURNING ");

    tokio::task::spawn_blocking(move || -> Result<ExecuteResult, DbError> {
        conn.with(|c| {
            let bound: Vec<SqlValue> = params.iter().map(json_param_to_sql).collect();
            let bound_refs: Vec<&dyn rusqlite::ToSql> =
                bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

            if has_returning {
                let mut stmt = c.prepare(&sql).map_err(map_err)?;
                let columns: Vec<ColumnMeta> = stmt
                    .columns()
                    .into_iter()
                    .map(|col| ColumnMeta {
                        name: col.name().to_string(),
                        ty: col.decl_type().unwrap_or("").to_string(),
                    })
                    .collect();
                let n = columns.len();
                let mut returned: Vec<Row> = Vec::new();
                let mut rows = stmt.query(bound_refs.as_slice()).map_err(map_err)?;
                while let Some(row) = rows.next().map_err(map_err)? {
                    let mut vals = Vec::with_capacity(n);
                    for i in 0..n {
                        vals.push(row_value_at(row, i)?);
                    }
                    returned.push(Row(vals));
                }
                // last_insert_rowid() is sticky per-connection: it retains
                // the rowid from any prior INSERT on this physical connection
                // and survives intervening UPDATE/DELETE. The pool reuses
                // connections, so a non-INSERT statement here would otherwise
                // report a stale rowid from someone else's earlier INSERT.
                let last_insert_id = if is_insert(&sql) {
                    let r = c.last_insert_rowid();
                    if r != 0 {
                        Some(r.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok(ExecuteResult {
                    affected_rows: returned.len() as u64,
                    last_insert_id,
                    returned_rows: returned,
                    returned_columns: columns,
                })
            } else {
                // rusqlite's `Connection::execute` returns `ExecuteReturnedResults`
                // when the SQL produces rows (e.g. a SELECT). Postgres' tokio-postgres
                // and mysql_async accept SELECT-via-execute and report 0 affected
                // rows; we normalize sqlite to the same contract by draining the
                // statement via `prepare + query` instead of failing.
                match c.execute(&sql, bound_refs.as_slice()) {
                    Ok(affected) => {
                        // Same sticky-rowid story as the RETURNING branch
                        // above: only propagate last_insert_rowid when this
                        // statement was actually an INSERT.
                        let last_insert_id = if is_insert(&sql) {
                            let r = c.last_insert_rowid();
                            if r != 0 {
                                Some(r.to_string())
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        Ok(ExecuteResult {
                            affected_rows: affected as u64,
                            last_insert_id,
                            returned_rows: vec![],
                            returned_columns: vec![],
                        })
                    }
                    Err(rusqlite::Error::ExecuteReturnedResults) => {
                        // Drain the rows so the statement actually runs (and any
                        // side effects in the user SQL fire) but discard them —
                        // execute()'s response shape doesn't carry result rows
                        // unless `returning` was set.
                        let mut stmt = c.prepare(&sql).map_err(map_err)?;
                        let mut rows = stmt.query(bound_refs.as_slice()).map_err(map_err)?;
                        while rows.next().map_err(map_err)?.is_some() {}
                        Ok(ExecuteResult {
                            affected_rows: 0,
                            last_insert_id: None,
                            returned_rows: vec![],
                            returned_columns: vec![],
                        })
                    }
                    Err(e) => Err(map_err(e)),
                }
            }
        })
    })
    .await
    .map_err(|e| DbError::DriverError {
        driver: "sqlite".into(),
        code: None,
        message: format!("spawn_blocking join: {e}"),
        failed_index: None,
    })?
}

/// Returns an `Err(DbError::DriverError {..})` carrying `failed_index` set
/// to the 0-based index of the failing statement. The handler layer in
/// `handlers::transaction` reads this directly to build the spec's
/// `{committed: false, failed_index, error}` envelope.
pub async fn transaction(
    pool: &SqlitePool,
    statements: Vec<TxStatement>,
    isolation: Option<Isolation>,
) -> Result<Vec<TxStepResult>, DbError> {
    let conn = pool.acquire().await?;

    tokio::task::spawn_blocking(move || -> Result<Vec<TxStepResult>, DbError> {
        let mut conn = conn;
        conn.with_mut(|c| {
            let begin_sql = match isolation {
                Some(Isolation::Serializable) => "BEGIN IMMEDIATE",
                Some(Isolation::ReadCommitted) | Some(Isolation::RepeatableRead) => {
                    tracing::warn!(
                        "sqlite ignores requested isolation; using BEGIN DEFERRED (always serializable in practice)"
                    );
                    "BEGIN DEFERRED"
                }
                None => "BEGIN DEFERRED",
            };
            c.execute_batch(begin_sql).map_err(map_err)?;

            let inner = run_tx_steps(c, &statements);
            match inner {
                Ok(results) => {
                    c.execute_batch("COMMIT").map_err(|e| {
                        // COMMIT failed: best-effort rollback to release the
                        // implicit txn on the pooled connection.
                        let _ = c.execute_batch("ROLLBACK");
                        map_err(e)
                    })?;
                    Ok(results)
                }
                Err(e) => {
                    // Best-effort rollback; ignore rollback errors (e.g. txn
                    // already aborted by SQLite).
                    let _ = c.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        })
    })
    .await
    .map_err(|e| DbError::DriverError {
        driver: "sqlite".into(),
        code: None,
        message: format!("spawn_blocking join: {e}"),
        failed_index: None,
    })?
}

fn step_err(idx: usize, e: rusqlite::Error) -> DbError {
    let code = match &e {
        rusqlite::Error::SqliteFailure(f, _) => Some(format!("{:?}", f.code)),
        _ => None,
    };
    DbError::DriverError {
        driver: "sqlite".into(),
        code,
        message: e.to_string(),
        failed_index: Some(idx),
    }
}

/// Execute the body of a transaction (after BEGIN, before COMMIT/ROLLBACK).
/// On error, returns Err so the caller can issue an explicit ROLLBACK.
fn run_tx_steps(
    c: &mut rusqlite::Connection,
    statements: &[TxStatement],
) -> Result<Vec<TxStepResult>, DbError> {
    let mut results: Vec<TxStepResult> = Vec::with_capacity(statements.len());

    for (idx, stmt) in statements.iter().enumerate() {
        let bound: Vec<SqlValue> = stmt.params.iter().map(json_param_to_sql).collect();
        let bound_refs: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

        let upper = stmt.sql.to_ascii_uppercase();
        let is_returning = upper.contains(" RETURNING ");
        let is_select = upper.trim_start().starts_with("SELECT");

        if is_select || is_returning {
            let mut prepared = c.prepare(&stmt.sql).map_err(|e| step_err(idx, e))?;
            let n = prepared.columns().len();
            let mut rows_out: Vec<Row> = Vec::new();
            let mut rows = prepared
                .query(bound_refs.as_slice())
                .map_err(|e| step_err(idx, e))?;
            while let Some(row) = rows.next().map_err(|e| step_err(idx, e))? {
                let mut vals = Vec::with_capacity(n);
                for i in 0..n {
                    vals.push(row_value_at(row, i)?);
                }
                rows_out.push(Row(vals));
            }
            results.push(TxStepResult {
                affected_rows: rows_out.len() as u64,
                rows: rows_out,
            });
        } else {
            let affected = c
                .execute(&stmt.sql, bound_refs.as_slice())
                .map_err(|e| step_err(idx, e))?;
            results.push(TxStepResult {
                affected_rows: affected as u64,
                rows: vec![],
            });
        }
    }
    Ok(results)
}

/// Run an arbitrary SELECT/RETURNING-bearing statement against a pinned
/// connection held in an Option slot (the registry's `PinnedConn::Sqlite`
/// variant). The slot is `.take()`-en to move the connection into
/// `spawn_blocking` and `.replace()`-d after the work completes.
///
/// The Option indirection lets us hand the connection to `spawn_blocking`
/// (which requires `'static`) without allocating a throwaway in-memory pool
/// just to satisfy `mem::replace`.
///
/// Note: SQLite re-prepares cheaply via its statement cache; the "handle"
/// in this driver is really a pinned connection rather than a server-side
/// plan. Callers pass the same SQL each time.
pub async fn run_prepared(
    conn_slot: &mut Option<crate::pool::sqlite::SqliteConn>,
    sql: &str,
    params: &[JsonParam],
) -> Result<QueryResult, DbError> {
    let owned = conn_slot.take().ok_or_else(|| DbError::DriverError {
        driver: "sqlite".into(),
        code: None,
        message: "pinned connection already taken (concurrent run_prepared?)".into(),
        failed_index: None,
    })?;
    let sql = sql.to_string();
    let params = params.to_vec();

    let (result, returned) = tokio::task::spawn_blocking(
        move || -> (Result<QueryResult, DbError>, crate::pool::sqlite::SqliteConn) {
            let mut owned = owned;
            let result = owned.with_mut(|c| -> Result<QueryResult, DbError> {
                let bound: Vec<SqlValue> = params.iter().map(json_param_to_sql).collect();
                let bound_refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut stmt = c.prepare(&sql).map_err(map_err)?;
                let columns: Vec<ColumnMeta> = stmt
                    .columns()
                    .into_iter()
                    .map(|col| ColumnMeta {
                        name: col.name().to_string(),
                        ty: col.decl_type().unwrap_or("").to_string(),
                    })
                    .collect();
                let n = columns.len();
                let mut rows_out: Vec<Row> = Vec::new();
                let mut rows = stmt.query(bound_refs.as_slice()).map_err(map_err)?;
                while let Some(row) = rows.next().map_err(map_err)? {
                    let mut vals = Vec::with_capacity(n);
                    for i in 0..n {
                        vals.push(row_value_at(row, i)?);
                    }
                    rows_out.push(Row(vals));
                }
                Ok(QueryResult {
                    columns,
                    rows: rows_out,
                })
            });
            (result, owned)
        },
    )
    .await
    .map_err(|e| DbError::DriverError {
        driver: "sqlite".into(),
        code: None,
        message: format!("spawn_blocking join: {e}"),
        failed_index: None,
    })?;

    *conn_slot = Some(returned);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::value::{JsonParam, RowValue};

    async fn pool() -> SqlitePool {
        SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_returns_rows_and_columns() {
        let p = pool().await;
        let setup = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            setup.with(|c| {
                c.execute_batch(
                    "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
                     INSERT INTO t (id, name) VALUES (1, 'alice'), (2, 'bob');",
                )
            })
        })
        .await
        .unwrap()
        .unwrap();

        let result = query(&p, "SELECT id, name FROM t ORDER BY id", &[], 30_000)
            .await
            .unwrap();
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[1].name, "name");
        assert_eq!(result.rows.len(), 2);
        assert!(matches!(&result.rows[0].0[0], RowValue::Int(1)));
        assert!(matches!(&result.rows[0].0[1], RowValue::Text(s) if s == "alice"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_with_positional_params() {
        let p = pool().await;
        let setup = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            setup.with(|c| {
                c.execute_batch("CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (1),(2),(3);")
            })
        })
        .await
        .unwrap()
        .unwrap();

        let r = query(
            &p,
            "SELECT n FROM t WHERE n > ? ORDER BY n",
            &[JsonParam::Int(1)],
            30_000,
        )
        .await
        .unwrap();
        assert_eq!(r.rows.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_returns_null_for_null_columns() {
        let p = pool().await;
        let setup = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            setup.with(|c| c.execute_batch("CREATE TABLE t (x TEXT); INSERT INTO t VALUES (NULL);"))
        })
        .await
        .unwrap()
        .unwrap();

        let r = query(&p, "SELECT x FROM t", &[], 30_000).await.unwrap();
        assert_eq!(r.rows.len(), 1);
        assert!(matches!(r.rows[0].0[0], RowValue::Null));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_sql_returns_driver_error() {
        let p = pool().await;
        let err = query(&p, "SELEKT 1", &[], 30_000).await.unwrap_err();
        match err {
            DbError::DriverError { driver, .. } => assert_eq!(driver, "sqlite"),
            other => panic!("expected DriverError, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_insert_reports_affected_and_last_insert_id() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            s.with(|c| c.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, n INT);"))
        })
        .await
        .unwrap()
        .unwrap();

        let r = execute(
            &p,
            "INSERT INTO t (n) VALUES (?), (?)",
            &[JsonParam::Int(1), JsonParam::Int(2)],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(r.affected_rows, 2);
        assert_eq!(r.last_insert_id.as_deref(), Some("2"));
        assert!(r.returned_rows.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_with_returning_populates_returned_rows() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            s.with(|c| c.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, n INT);"))
        })
        .await
        .unwrap()
        .unwrap();

        let r = execute(
            &p,
            "INSERT INTO t (n) VALUES (?) RETURNING id, n",
            &[JsonParam::Int(7)],
            &["id".into(), "n".into()],
        )
        .await
        .unwrap();
        assert_eq!(r.returned_rows.len(), 1);
        assert_eq!(r.returned_columns.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_rejects_multi_statement_sql() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || s.with(|c| c.execute_batch("CREATE TABLE t (n INT);")))
            .await
            .unwrap()
            .unwrap();
        let err = execute(
            &p,
            "INSERT INTO t VALUES (1); INSERT INTO t VALUES (2)",
            &[],
            &[],
        )
        .await
        .unwrap_err();
        match err {
            DbError::DriverError { driver, code, .. } => {
                assert_eq!(driver, "sqlite");
                assert_eq!(code.as_deref(), Some("MULTI_STATEMENT"));
            }
            other => panic!("expected DriverError, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_with_select_returns_zero_affected_rows() {
        // Symmetry with postgres + mysql drivers: execute(SELECT) must not throw.
        // Postgres' tokio-postgres and mysql's mysql_async accept SELECT through
        // their execute paths and report 0 affected rows; rusqlite's
        // Connection::execute returns ExecuteReturnedResults instead, which the
        // worker now intercepts and normalizes.
        let p = pool().await;
        let r = execute(&p, "SELECT 1 AS v", &[], &[]).await.unwrap();
        assert_eq!(r.affected_rows, 0);
        assert!(r.returned_rows.is_empty());
        assert!(r.returned_columns.is_empty());
        assert!(r.last_insert_id.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_allows_trailing_semicolon() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || s.with(|c| c.execute_batch("CREATE TABLE t (n INT);")))
            .await
            .unwrap()
            .unwrap();
        // Trailing `;` and whitespace must not trigger multi-statement detection.
        let r = execute(&p, "INSERT INTO t VALUES (1);   ", &[], &[])
            .await
            .unwrap();
        assert_eq!(r.affected_rows, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_update_reports_affected_only() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            s.with(|c| c.execute_batch("CREATE TABLE t (n INT); INSERT INTO t VALUES (1),(2),(3);"))
        })
        .await
        .unwrap()
        .unwrap();

        let r = execute(
            &p,
            "UPDATE t SET n = n + 10 WHERE n > ?",
            &[JsonParam::Int(1)],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(r.affected_rows, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn transaction_commits_when_all_statements_succeed() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || s.with(|c| c.execute_batch("CREATE TABLE t (n INT);")))
            .await
            .unwrap()
            .unwrap();

        let stmts = vec![
            TxStatement {
                sql: "INSERT INTO t VALUES (?)".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "INSERT INTO t VALUES (?)".into(),
                params: vec![JsonParam::Int(2)],
            },
        ];
        let res = transaction(&p, stmts, None).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].affected_rows, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn transaction_rolls_back_on_failure_and_returns_failed_index() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            s.with(|c| c.execute_batch("CREATE TABLE t (n INT NOT NULL);"))
        })
        .await
        .unwrap()
        .unwrap();

        let stmts = vec![
            TxStatement {
                sql: "INSERT INTO t VALUES (?)".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "INSERT INTO t VALUES (?)".into(),
                params: vec![JsonParam::Null], // violates NOT NULL
            },
        ];
        let err = transaction(&p, stmts, None).await.unwrap_err();
        match err {
            DbError::DriverError {
                driver,
                message,
                failed_index,
                ..
            } => {
                assert_eq!(driver, "sqlite");
                assert_eq!(failed_index, Some(1));
                assert!(
                    message.contains("NOT NULL") || message.contains("constraint"),
                    "got: {message}"
                );
            }
            other => panic!("expected DriverError, got {other:?}"),
        }

        // Verify rollback: table should be empty.
        let r = query(&p, "SELECT COUNT(*) FROM t", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(&r.rows[0].0[0], RowValue::Int(0)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn transaction_serializable_uses_begin_immediate() {
        // Smoke: running with Serializable should not error on SQLite.
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || s.with(|c| c.execute_batch("CREATE TABLE t (n INT);")))
            .await
            .unwrap()
            .unwrap();

        let stmts = vec![TxStatement {
            sql: "INSERT INTO t VALUES (1)".into(),
            params: vec![],
        }];
        let res = transaction(&p, stmts, Some(Isolation::Serializable))
            .await
            .unwrap();
        assert_eq!(res.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_then_run_executes_with_params() {
        let p = pool().await;
        let s = p.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            s.with(|c| {
                c.execute_batch("CREATE TABLE t (n INT); INSERT INTO t VALUES (10),(20),(30);")
            })
        })
        .await
        .unwrap()
        .unwrap();

        let mut conn_slot = Some(p.acquire().await.unwrap());
        let result = run_prepared(
            &mut conn_slot,
            "SELECT n FROM t WHERE n > ? ORDER BY n",
            &[JsonParam::Int(15)],
        )
        .await
        .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert!(
            conn_slot.is_some(),
            "conn should be returned to the slot after run_prepared"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_prepared_returns_error_when_conn_slot_empty() {
        // Defends the race-guard at the top of `run_prepared`: if two callers
        // hit the same registry entry concurrently, the second `.take()` sees
        // None and must return a DriverError rather than panicking.
        let mut empty: Option<crate::pool::sqlite::SqliteConn> = None;
        let err = run_prepared(&mut empty, "SELECT 1", &[]).await.unwrap_err();
        match err {
            DbError::DriverError {
                driver, message, ..
            } => {
                assert_eq!(driver, "sqlite");
                assert!(
                    message.contains("already taken") || message.contains("pinned"),
                    "got: {message}"
                );
            }
            other => panic!("expected DriverError, got {other:?}"),
        }
    }
}
