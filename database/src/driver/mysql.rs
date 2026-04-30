//! MySQL driver: query/execute/transaction/prepare.

use crate::driver::{
    ColumnMeta, ExecuteResult, Isolation, QueryResult, Row, TxStatement, TxStepResult,
};
use crate::error::DbError;
use crate::pool::MysqlPool;
use crate::value::{JsonParam, RowValue};
use mysql_async::prelude::Queryable;
use mysql_async::{params::Params, Value as MyValue};
use std::time::Duration;

pub async fn query(
    pool: &MysqlPool,
    sql: &str,
    params: &[JsonParam],
    timeout_ms: u64,
) -> Result<QueryResult, DbError> {
    let mut conn = pool.acquire().await?;
    let bound = bind_params(params);
    let fut = conn.exec_iter(sql, bound);
    let mut result = tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
        .await
        .map_err(|_| DbError::QueryTimeout {
            db: "(mysql)".into(),
            timeout_ms,
        })?
        .map_err(map_err)?;

    let cols: Vec<ColumnMeta> = result
        .columns_ref()
        .iter()
        .map(|c| ColumnMeta {
            name: c.name_str().to_string(),
            ty: format!("{:?}", c.column_type()),
        })
        .collect();

    let raw_rows: Vec<mysql_async::Row> = result.collect().await.map_err(map_err)?;
    let mut out_rows: Vec<Row> = Vec::with_capacity(raw_rows.len());
    for row in raw_rows {
        let cells = row_cells(&row);
        out_rows.push(Row(cells));
    }
    Ok(QueryResult {
        columns: cols,
        rows: out_rows,
    })
}

fn bind_params(params: &[JsonParam]) -> Params {
    let v: Vec<MyValue> = params.iter().map(json_param_to_my).collect();
    if v.is_empty() {
        Params::Empty
    } else {
        Params::Positional(v)
    }
}

fn json_param_to_my(p: &JsonParam) -> MyValue {
    match p {
        JsonParam::Null => MyValue::NULL,
        JsonParam::Bool(b) => MyValue::Int(if *b { 1 } else { 0 }),
        JsonParam::Int(i) => MyValue::Int(*i),
        JsonParam::Float(f) => MyValue::Double(*f),
        JsonParam::Text(s) => MyValue::Bytes(s.as_bytes().to_vec()),
        JsonParam::Json(v) => MyValue::Bytes(v.to_string().into_bytes()),
    }
}

fn row_cells(row: &mysql_async::Row) -> Vec<RowValue> {
    let mut cells = Vec::with_capacity(row.columns_ref().len());
    for i in 0..row.columns_ref().len() {
        let v: MyValue = row.as_ref(i).cloned().unwrap_or(MyValue::NULL);
        cells.push(my_to_row_value(v));
    }
    cells
}

fn my_to_row_value(v: MyValue) -> RowValue {
    match v {
        MyValue::NULL => RowValue::Null,
        MyValue::Int(i) => RowValue::Int(i),
        MyValue::UInt(u) => {
            if u <= i64::MAX as u64 {
                RowValue::Int(u as i64)
            } else {
                RowValue::Decimal(u.to_string())
            }
        }
        MyValue::Float(f) => RowValue::Float(f as f64),
        MyValue::Double(f) => RowValue::Float(f),
        MyValue::Bytes(b) => match std::str::from_utf8(&b) {
            Ok(s) => RowValue::Text(s.to_string()),
            Err(_) => RowValue::Bytes(b),
        },
        MyValue::Date(y, mo, d, h, mi, s, _us) => {
            use chrono::{TimeZone, Utc};
            match Utc.with_ymd_and_hms(y as i32, mo as u32, d as u32, h as u32, mi as u32, s as u32)
            {
                chrono::LocalResult::Single(t) => RowValue::Timestamp(t),
                _ => RowValue::Null,
            }
        }
        MyValue::Time(_, _, _, _, _, _) => RowValue::Text(format!("{v:?}")),
    }
}

pub(crate) fn map_err(e: mysql_async::Error) -> DbError {
    let code = match &e {
        mysql_async::Error::Server(s) => Some(s.code.to_string()),
        _ => None,
    };
    DbError::DriverError {
        driver: "mysql".into(),
        code,
        message: e.to_string(),
        failed_index: None,
    }
}

pub async fn execute(
    pool: &MysqlPool,
    sql: &str,
    params: &[JsonParam],
    returning: &[String],
) -> Result<ExecuteResult, DbError> {
    if !returning.is_empty() {
        tracing::warn!(
            driver = "mysql",
            "RETURNING not supported on MySQL; ignoring `returning` array"
        );
    }
    let mut conn = pool.acquire().await?;
    let bound = bind_params(params);
    conn.exec_drop(sql, bound).await.map_err(map_err)?;
    let affected = conn.affected_rows();
    // mysql_async's last_insert_id() is sticky per-connection: an UPDATE on
    // a pool-reused connection that previously ran an INSERT will still
    // return Some(prior_id). Gate on the SQL prefix so non-INSERTs always
    // report None.
    let last_insert_id = if is_insert(sql) {
        conn.last_insert_id().map(|i| i.to_string())
    } else {
        None
    };
    Ok(ExecuteResult {
        affected_rows: affected,
        last_insert_id,
        returned_rows: vec![],
        returned_columns: vec![],
    })
}

/// Same naïve prefix check as `driver::sqlite::is_insert`. False-negatives on
/// `REPLACE INTO …` and CTE-prefixed INSERTs fall through to
/// `last_insert_id: None`, which is safer than reporting a stale id from a
/// pool-reused connection.
fn is_insert(sql: &str) -> bool {
    sql.trim_start().to_ascii_uppercase().starts_with("INSERT")
}

pub async fn transaction(
    pool: &MysqlPool,
    statements: Vec<TxStatement>,
    isolation: Option<Isolation>,
) -> Result<Vec<TxStepResult>, DbError> {
    let mut conn = pool.acquire().await?;
    let iso_sql = match isolation {
        Some(Isolation::ReadCommitted) => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        Some(Isolation::RepeatableRead) => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
        Some(Isolation::Serializable) => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
        None => "",
    };
    if !iso_sql.is_empty() {
        conn.query_drop(iso_sql).await.map_err(map_err)?;
    }
    conn.query_drop("START TRANSACTION")
        .await
        .map_err(map_err)?;

    let mut results: Vec<TxStepResult> = Vec::with_capacity(statements.len());

    for (idx, stmt) in statements.iter().enumerate() {
        let upper = stmt.sql.to_ascii_uppercase();
        let returns_rows = upper.trim_start().starts_with("SELECT");
        let bound = bind_params(&stmt.params);

        let step_result: Result<TxStepResult, DbError> = if returns_rows {
            match conn.exec_iter(stmt.sql.as_str(), bound).await {
                Ok(mut iter) => {
                    let raw: Result<Vec<mysql_async::Row>, _> = iter.collect().await;
                    match raw {
                        Ok(raw_rows) => {
                            let cells_rows: Vec<Row> =
                                raw_rows.iter().map(|r| Row(row_cells(r))).collect();
                            Ok(TxStepResult {
                                affected_rows: cells_rows.len() as u64,
                                rows: cells_rows,
                            })
                        }
                        Err(e) => Err(step_err(idx, e)),
                    }
                }
                Err(e) => Err(step_err(idx, e)),
            }
        } else {
            match conn.exec_drop(stmt.sql.as_str(), bound).await {
                Ok(_) => Ok(TxStepResult {
                    affected_rows: conn.affected_rows(),
                    rows: vec![],
                }),
                Err(e) => Err(step_err(idx, e)),
            }
        };
        match step_result {
            Ok(s) => results.push(s),
            Err(e) => {
                let _ = conn.query_drop("ROLLBACK").await;
                return Err(e);
            }
        }
    }
    conn.query_drop("COMMIT").await.map_err(map_err)?;
    Ok(results)
}

fn step_err(idx: usize, e: mysql_async::Error) -> DbError {
    DbError::DriverError {
        driver: "mysql".into(),
        code: match &e {
            mysql_async::Error::Server(s) => Some(s.code.to_string()),
            _ => None,
        },
        message: e.to_string(),
        failed_index: Some(idx),
    }
}

pub async fn run_prepared(
    conn: &mut crate::pool::mysql::MysqlConn,
    sql: &str,
    params: &[JsonParam],
) -> Result<QueryResult, DbError> {
    let bound = bind_params(params);
    let mut iter = conn.exec_iter(sql, bound).await.map_err(map_err)?;
    let cols: Vec<ColumnMeta> = iter
        .columns_ref()
        .iter()
        .map(|c| ColumnMeta {
            name: c.name_str().to_string(),
            ty: format!("{:?}", c.column_type()),
        })
        .collect();
    let raw_rows: Vec<mysql_async::Row> = iter.collect().await.map_err(map_err)?;
    let out_rows: Vec<Row> = raw_rows.iter().map(|r| Row(row_cells(r))).collect();
    Ok(QueryResult {
        columns: cols,
        rows: out_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::pool::MysqlPool;
    use crate::value::{JsonParam, RowValue};

    fn url() -> Option<String> {
        std::env::var("TEST_MYSQL_URL").ok()
    }

    async fn pool() -> Option<MysqlPool> {
        let tls = crate::config::TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ca_cert: None,
        };
        Some(MysqlPool::new(&url()?, &PoolConfig::default(), &tls).unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_query_returns_int_text_null() {
        let Some(p) = pool().await else { return };
        let r = query(&p, "SELECT 1 AS a, 'x' AS b, NULL AS c", &[], 30_000)
            .await
            .unwrap();
        assert_eq!(r.columns.len(), 3);
        assert!(matches!(
            &r.rows[0].0[0],
            RowValue::Int(1) | RowValue::BigInt(1)
        ));
        assert!(matches!(&r.rows[0].0[1], RowValue::Text(s) if s == "x"));
        assert!(matches!(&r.rows[0].0[2], RowValue::Null));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_query_with_positional_params() {
        let Some(p) = pool().await else { return };
        let r = query(
            &p,
            "SELECT ? + ? AS sum",
            &[JsonParam::Int(40), JsonParam::Int(2)],
            30_000,
        )
        .await
        .unwrap();
        // MySQL types `?+?` as MYSQL_TYPE_DOUBLE (parameter placeholders carry no
        // declared type, so the optimizer picks DOUBLE for the result column).
        // Accept any numeric variant equal to 42 — the test asserts "positional
        // params bind correctly", not "integer arithmetic preserves type".
        let v = &r.rows[0].0[0];
        let ok = match v {
            RowValue::Int(42) | RowValue::BigInt(42) => true,
            RowValue::Float(f) => (f - 42.0).abs() < 1e-9,
            _ => false,
        };
        assert!(ok, "expected ~42, got {v:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_execute_insert_reports_affected_and_last_insert_id() {
        let Some(p) = pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_t", &[], &[]).await;
        let _ = execute(
            &p,
            "CREATE TABLE db_w_t (id INT AUTO_INCREMENT PRIMARY KEY, n INT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        let r = execute(
            &p,
            "INSERT INTO db_w_t (n) VALUES (?), (?)",
            &[JsonParam::Int(1), JsonParam::Int(2)],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(r.affected_rows, 2);
        assert!(r.last_insert_id.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_execute_with_returning_warns_and_ignores() {
        let Some(p) = pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_t2", &[], &[]).await;
        let _ = execute(
            &p,
            "CREATE TABLE db_w_t2 (id INT AUTO_INCREMENT PRIMARY KEY, n INT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        let r = execute(
            &p,
            "INSERT INTO db_w_t2 (n) VALUES (?)",
            &[JsonParam::Int(7)],
            &["id".into()],
        )
        .await
        .unwrap();
        assert_eq!(r.affected_rows, 1);
        assert!(r.returned_rows.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_transaction_commits() {
        let Some(p) = pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_tx", &[], &[]).await;
        let _ = execute(&p, "CREATE TABLE db_w_tx (n INT)", &[], &[])
            .await
            .unwrap();
        let stmts = vec![
            TxStatement {
                sql: "INSERT INTO db_w_tx VALUES (?)".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "INSERT INTO db_w_tx VALUES (?)".into(),
                params: vec![JsonParam::Int(2)],
            },
        ];
        let res = transaction(&p, stmts, Some(Isolation::RepeatableRead))
            .await
            .unwrap();
        assert_eq!(res.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_transaction_rolls_back_on_failure() {
        let Some(p) = pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_tx2", &[], &[]).await;
        let _ = execute(&p, "CREATE TABLE db_w_tx2 (n INT NOT NULL)", &[], &[])
            .await
            .unwrap();
        let stmts = vec![
            TxStatement {
                sql: "INSERT INTO db_w_tx2 VALUES (?)".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "INSERT INTO db_w_tx2 VALUES (?)".into(),
                params: vec![JsonParam::Null],
            },
        ];
        let err = transaction(&p, stmts, None).await.unwrap_err();
        assert!(matches!(err, DbError::DriverError { .. }));
        let r = query(&p, "SELECT COUNT(*) AS c FROM db_w_tx2", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(
            &r.rows[0].0[0],
            RowValue::Int(0) | RowValue::BigInt(0)
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_run_prepared_executes_with_params() {
        let Some(p) = pool().await else { return };
        let mut conn = p.acquire().await.unwrap();
        let result = run_prepared(
            &mut conn,
            "SELECT ? + ? AS total",
            &[JsonParam::Int(40), JsonParam::Int(2)],
        )
        .await
        .unwrap();
        // See note in `my_query_with_positional_params`: `?+?` returns DOUBLE.
        let v = &result.rows[0].0[0];
        let ok = match v {
            RowValue::Int(42) | RowValue::BigInt(42) => true,
            RowValue::Float(f) => (f - 42.0).abs() < 1e-9,
            _ => false,
        };
        assert!(ok, "expected ~42, got {v:?}");
    }
}
