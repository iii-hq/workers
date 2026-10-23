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
    opts: crate::driver::ReadOptions,
) -> Result<QueryResult, DbError> {
    let timeout_ms = opts.timeout_ms;
    let mut conn = pool.acquire().await?;
    let started = std::time::Instant::now();
    let bound = bind_params(params);

    // mysql_async's `exec_iter` resolves once the server returns column
    // metadata; row payloads are streamed lazily. Wrapping only `exec_iter`
    // would leave row collection unbounded — a slow-row query (e.g.,
    // per-row SLEEP, or a slow scan) would silently bypass `timeout_ms`.
    // Wrap the whole prepare→stream pipeline.
    let work = async {
        if let Some(max) = opts.max_rows {
            // The server stops a top-level SELECT after max+1 rows without
            // the SQL being rewritten (an explicit LIMIT in it still wins;
            // the client cap below covers that). Session-scoped; the pool
            // resets the connection when it comes back.
            conn.query_drop(format!(
                "SET SESSION sql_select_limit = {}",
                max.saturating_add(1)
            ))
            .await
            .map_err(map_err)?;
        }
        // `database::query` is the READ surface a narrowed agent policy
        // grants; the server enforces it: SET TRANSACTION READ ONLY applies
        // to the statement's implicit transaction, so a write fails with
        // ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION instead of landing. The
        // flag is consumed by that transaction — the pooled conn reverts to
        // normal afterwards. A prepared statement is always one statement.
        conn.query_drop("SET TRANSACTION READ ONLY")
            .await
            .map_err(map_err)?;
        let mut result = conn.exec_iter(sql, bound).await.map_err(map_err)?;
        let cols: Vec<ColumnMeta> = result
            .columns_ref()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name_str().to_string(),
                ty: sql_type_name(c),
            })
            .collect();
        let (raw_rows, truncated) = match opts.max_rows {
            None => (
                result
                    .collect::<mysql_async::Row>()
                    .await
                    .map_err(map_err)?,
                false,
            ),
            Some(max) => {
                let mut rows = Vec::new();
                let mut truncated = false;
                while let Some(row) = result.next().await.map_err(map_err)? {
                    if rows.len() as u64 >= max {
                        truncated = true;
                        break;
                    }
                    rows.push(row);
                }
                // Whatever the server still sends is read off the wire and
                // dropped, never materialized.
                drop(result);
                (rows, truncated)
            }
        };
        if opts.max_rows.is_some() {
            let _ = conn
                .query_drop("SET SESSION sql_select_limit = DEFAULT")
                .await;
        }
        Ok::<_, DbError>((cols, raw_rows, truncated))
    };
    let (cols, raw_rows, truncated) = tokio::time::timeout(Duration::from_millis(timeout_ms), work)
        .await
        .map_err(|_| DbError::QueryTimeout {
            db: "(mysql)".into(),
            timeout_ms,
        })??;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let mut out_rows: Vec<Row> = Vec::with_capacity(raw_rows.len());
    for row in raw_rows {
        let cells = row_cells(&row);
        out_rows.push(Row(cells));
    }
    Ok(QueryResult {
        columns: cols,
        rows: out_rows,
        truncated,
        elapsed_ms,
    })
}

/// MySQL's binary charset. Wire metadata reuses one protocol type for a text
/// column and its binary twin (`TEXT`/`BLOB`, `VARCHAR`/`VARBINARY`); the
/// charset is the only thing that separates them.
const BINARY_CHARSET: u16 = 63;

/// The SQL type name for a result column, as a caller would write it.
///
/// The protocol hands back an internal enum, and `{:?}` on it produced
/// `MYSQL_TYPE_VAR_STRING` — leaked into the console's grid header and into
/// every agent reading `columns[].type`. Postgres reports `text` and SQLite
/// reports the declared type, so MySQL was the odd one out; this restores
/// parity. Length and precision are not on the wire, so `varchar` here where
/// `describeTable` (which reads the catalog) says `varchar(50)`.
fn sql_type_name(col: &mysql_async::Column) -> String {
    use mysql_async::consts::{ColumnFlags, ColumnType::*};

    let binary = col.character_set() == BINARY_CHARSET;
    // ENUM and SET travel as strings; only the flags tell them apart.
    if col.flags().contains(ColumnFlags::ENUM_FLAG) {
        return "enum".into();
    }
    if col.flags().contains(ColumnFlags::SET_FLAG) {
        return "set".into();
    }
    let name = match col.column_type() {
        MYSQL_TYPE_TINY => "tinyint",
        MYSQL_TYPE_SHORT => "smallint",
        MYSQL_TYPE_INT24 => "mediumint",
        MYSQL_TYPE_LONG => "int",
        MYSQL_TYPE_LONGLONG => "bigint",
        MYSQL_TYPE_FLOAT => "float",
        MYSQL_TYPE_DOUBLE => "double",
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
        MYSQL_TYPE_BIT => "bit",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        MYSQL_TYPE_YEAR => "year",
        MYSQL_TYPE_JSON => "json",
        MYSQL_TYPE_ENUM => "enum",
        MYSQL_TYPE_SET => "set",
        MYSQL_TYPE_GEOMETRY => "geometry",
        MYSQL_TYPE_NULL => "null",
        MYSQL_TYPE_TINY_BLOB => return blob_or_text(binary, "tiny"),
        MYSQL_TYPE_MEDIUM_BLOB => return blob_or_text(binary, "medium"),
        MYSQL_TYPE_LONG_BLOB => return blob_or_text(binary, "long"),
        MYSQL_TYPE_BLOB => return blob_or_text(binary, ""),
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING => {
            if binary {
                "varbinary"
            } else {
                "varchar"
            }
        }
        MYSQL_TYPE_STRING => {
            if binary {
                "binary"
            } else {
                "char"
            }
        }
        // `MYSQL_TYPE_UNKNOWN` and anything a future server adds. Better a
        // lowercase debug name than a wrong guess at a SQL type.
        other => return format!("{other:?}").to_ascii_lowercase(),
    };

    if col.flags().contains(ColumnFlags::UNSIGNED_FLAG) && is_integer(col.column_type()) {
        format!("{name} unsigned")
    } else {
        name.to_string()
    }
}

fn blob_or_text(binary: bool, size: &str) -> String {
    format!("{size}{}", if binary { "blob" } else { "text" })
}

fn is_integer(ty: mysql_async::consts::ColumnType) -> bool {
    use mysql_async::consts::ColumnType::*;
    matches!(
        ty,
        MYSQL_TYPE_TINY
            | MYSQL_TYPE_SHORT
            | MYSQL_TYPE_INT24
            | MYSQL_TYPE_LONG
            | MYSQL_TYPE_LONGLONG
    )
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
    let columns = row.columns_ref();
    let mut cells = Vec::with_capacity(columns.len());
    for (i, col) in columns.iter().enumerate() {
        let v: MyValue = row.as_ref(i).cloned().unwrap_or(MyValue::NULL);
        cells.push(my_cell(col, v));
    }
    cells
}

/// A result cell, typed by its column (the binary protocol alone cannot
/// tell `binary(4)` from text, a `date` from a `datetime`, or `json` from a
/// string):
///
/// | column | JSON |
/// |---|---|
/// | integers, `year` | number (`bigint unsigned`: string, it may exceed i64) |
/// | `decimal` | string |
/// | `float`, `double` | number |
/// | `date` | `"YYYY-MM-DD"` |
/// | `datetime`, `timestamp` | RFC 3339 with `Z`, fraction when non-zero (read as UTC, the shape postgres `timestamp` has) |
/// | `time` | `"[-]HH:MM:SS[.ffffff]"`, hours past 24 as MySQL prints them |
/// | `json` | the JSON value |
/// | `bit(n)` | string of n `0`/`1` |
/// | `binary`, `varbinary`, `blob`, `geometry` | base64 |
/// | `char`, `varchar`, `text`, `enum`, `set` | string (`set` as `a,b`) |
fn my_cell(col: &mysql_async::Column, v: MyValue) -> RowValue {
    use mysql_async::consts::{ColumnFlags, ColumnType::*};
    if matches!(v, MyValue::NULL) {
        return RowValue::Null;
    }
    let binary = col.character_set() == BINARY_CHARSET;
    let unsigned = col.flags().contains(ColumnFlags::UNSIGNED_FLAG);
    match col.column_type() {
        MYSQL_TYPE_LONGLONG if unsigned => match v {
            MyValue::UInt(u) => RowValue::Decimal(u.to_string()),
            MyValue::Int(i) => RowValue::Decimal(i.to_string()),
            other => my_to_row_value(other),
        },
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => match v {
            MyValue::Bytes(b) => RowValue::Decimal(String::from_utf8_lossy(&b).into_owned()),
            other => my_to_row_value(other),
        },
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => match v {
            MyValue::Date(y, mo, d, ..) => RowValue::Text(format!("{y:04}-{mo:02}-{d:02}")),
            other => my_to_row_value(other),
        },
        MYSQL_TYPE_DATETIME
        | MYSQL_TYPE_DATETIME2
        | MYSQL_TYPE_TIMESTAMP
        | MYSQL_TYPE_TIMESTAMP2 => my_to_row_value(v),
        MYSQL_TYPE_JSON => match v {
            MyValue::Bytes(b) => match serde_json::from_slice(&b) {
                Ok(json) => RowValue::Json(json),
                Err(_) => RowValue::Text(String::from_utf8_lossy(&b).into_owned()),
            },
            other => my_to_row_value(other),
        },
        MYSQL_TYPE_BIT => match v {
            MyValue::Bytes(b) => {
                let n = (col.column_length() as usize).clamp(1, b.len() * 8);
                let all: String = b.iter().map(|byte| format!("{byte:08b}")).collect();
                RowValue::Text(all[all.len() - n..].to_string())
            }
            MyValue::UInt(u) => RowValue::Text(format!("{u:b}")),
            MyValue::Int(i) => RowValue::Text(format!("{i:b}")),
            other => my_to_row_value(other),
        },
        MYSQL_TYPE_STRING
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
        | MYSQL_TYPE_BLOB
        | MYSQL_TYPE_GEOMETRY
            if binary =>
        {
            match v {
                MyValue::Bytes(b) => RowValue::Bytes(b),
                other => my_to_row_value(other),
            }
        }
        _ => my_to_row_value(v),
    }
}

/// `[-]HH:MM:SS[.ffffff]` — a MySQL TIME is a duration (−838:59:59 to
/// 838:59:59), printed the way MySQL prints it.
fn my_time(neg: bool, days: u32, h: u8, m: u8, s: u8, us: u32) -> String {
    let hours = u64::from(days) * 24 + u64::from(h);
    let mut out = format!("{}{hours:02}:{m:02}:{s:02}", if neg { "-" } else { "" });
    if us > 0 {
        out.push('.');
        out.push_str(format!("{us:06}").trim_end_matches('0'));
    }
    out
}

/// Also the binlog capture path's encoder (which has no column metadata),
/// so a primary key in a `row-changed` event reads like the same column in
/// `database::query` wherever the value alone decides.
pub(crate) fn my_to_row_value(v: MyValue) -> RowValue {
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
        MyValue::Bytes(b) => match String::from_utf8(b) {
            Ok(s) => RowValue::Text(s),
            Err(e) => RowValue::Bytes(e.into_bytes()),
        },
        MyValue::Date(y, mo, d, h, mi, s, us) => {
            use chrono::{NaiveDate, TimeZone, Utc};
            match NaiveDate::from_ymd_opt(y.into(), mo.into(), d.into())
                .and_then(|date| date.and_hms_micro_opt(h.into(), mi.into(), s.into(), us))
            {
                Some(naive) => RowValue::Timestamp(Utc.from_utc_datetime(&naive)),
                // Zero dates (`0000-00-00 …`) have no calendar value: as
                // MySQL prints them.
                None => RowValue::Text(format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")),
            }
        }
        MyValue::Time(neg, days, h, m, s, us) => RowValue::Text(my_time(neg, days, h, m, s, us)),
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
    let started = std::time::Instant::now();
    let bound = bind_params(params);
    conn.exec_drop(sql, bound).await.map_err(map_err)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
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
        elapsed_ms,
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
                    let columns = iter
                        .columns_ref()
                        .iter()
                        .map(|col| ColumnMeta {
                            name: col.name_str().to_string(),
                            ty: sql_type_name(col),
                        })
                        .collect();
                    let raw: Result<Vec<mysql_async::Row>, _> = iter.collect().await;
                    match raw {
                        Ok(raw_rows) => {
                            let cells_rows: Vec<Row> =
                                raw_rows.iter().map(|r| Row(row_cells(r))).collect();
                            Ok(TxStepResult {
                                affected_rows: cells_rows.len() as u64,
                                rows: cells_rows,
                                columns,
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
                    columns: vec![],
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

/// Issue `SET TRANSACTION ISOLATION LEVEL ...; START TRANSACTION` on a
/// pinned connection. Used by `beginTransaction`. MySQL needs the isolation
/// statement before `START TRANSACTION` (per-server-default otherwise).
pub async fn tx_begin(
    conn: &mut crate::pool::mysql::MysqlConn,
    isolation: Option<Isolation>,
) -> Result<(), DbError> {
    let iso_sql = match isolation {
        Some(Isolation::ReadCommitted) => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        Some(Isolation::RepeatableRead) => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
        Some(Isolation::Serializable) => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
        None => "",
    };
    if !iso_sql.is_empty() {
        conn.query_drop(iso_sql).await.map_err(map_err)?;
    }
    conn.query_drop("START TRANSACTION").await.map_err(map_err)
}

/// `COMMIT` the in-progress transaction on a pinned connection.
pub async fn tx_commit(conn: &mut crate::pool::mysql::MysqlConn) -> Result<(), DbError> {
    conn.query_drop("COMMIT").await.map_err(map_err)
}

/// `ROLLBACK` the in-progress transaction on a pinned connection. Callers
/// that want best-effort rollback should `let _ =` the result.
pub async fn tx_rollback(conn: &mut crate::pool::mysql::MysqlConn) -> Result<(), DbError> {
    conn.query_drop("ROLLBACK").await.map_err(map_err)
}

/// Run an INSERT/UPDATE/DELETE on a pinned connection inside an active
/// transaction. Mirrors `execute()` but without pool acquire. MySQL has
/// no `RETURNING`, so the `returning` array is logged-and-ignored as
/// usual. Used by `transactionExecute`.
pub async fn tx_execute(
    conn: &mut crate::pool::mysql::MysqlConn,
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
    let bound = bind_params(params);
    conn.exec_drop(sql, bound).await.map_err(map_err)?;
    let affected = conn.affected_rows();
    let last_insert_id = if is_insert(sql) {
        conn.last_insert_id().map(|i| i.to_string())
    } else {
        None
    };
    Ok(ExecuteResult {
        affected_rows: affected,
        last_insert_id,
        ..Default::default()
    })
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
            ty: sql_type_name(c),
        })
        .collect();
    let raw_rows: Vec<mysql_async::Row> = iter.collect().await.map_err(map_err)?;
    let out_rows: Vec<Row> = raw_rows.iter().map(|r| Row(row_cells(r))).collect();
    Ok(QueryResult {
        columns: cols,
        rows: out_rows,
        truncated: false,
        elapsed_ms: 0,
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
            ..Default::default()
        };
        Some(MysqlPool::new(&url()?, &PoolConfig::default(), &tls).unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_query_returns_int_text_null() {
        let Some(p) = pool().await else { return };
        let r = query(
            &p,
            "SELECT 1 AS a, 'x' AS b, NULL AS c",
            &[],
            crate::driver::ReadOptions {
                timeout_ms: 30_000,
                ..Default::default()
            },
        )
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
            crate::driver::ReadOptions {
                timeout_ms: 30_000,
                ..Default::default()
            },
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
        let r = query(
            &p,
            "SELECT COUNT(*) AS c FROM db_w_tx2",
            &[],
            crate::driver::ReadOptions {
                timeout_ms: 30_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            &r.rows[0].0[0],
            RowValue::Int(0) | RowValue::BigInt(0)
        ));
    }

    /// Regression: `tokio::time::timeout` must wrap both the `exec_iter`
    /// dispatch *and* the `result.collect()` row stream. mysql_async's
    /// `exec_iter` resolves once the server returns column metadata, so a
    /// query whose planner is fast but whose row stream is slow (here:
    /// `SELECT SLEEP(N)` per-row) would silently bypass the timeout if only
    /// `exec_iter` were wrapped. The fix wraps the whole pipeline; this test
    /// asserts the timeout fires.
    #[tokio::test(flavor = "multi_thread")]
    async fn my_query_timeout_applies_to_row_streaming() {
        let Some(p) = pool().await else { return };
        let start = std::time::Instant::now();
        // 3 rows × SLEEP(2) = ~6s of server-side row generation. With
        // timeout_ms = 500, the fix's whole-pipeline wrap fires within ~1s.
        // The buggy code would wait the full ~6s and return rows.
        let res = query(
            &p,
            "SELECT SLEEP(2), n FROM (SELECT 1 AS n UNION SELECT 2 UNION SELECT 3) AS t",
            &[],
            crate::driver::ReadOptions {
                timeout_ms: 500,
                ..Default::default()
            },
        )
        .await;
        let elapsed = start.elapsed();
        assert!(
            matches!(res, Err(DbError::QueryTimeout { .. })),
            "expected QueryTimeout, got {res:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "timeout should fire well before the ~6s row stream completes; elapsed={elapsed:?}"
        );
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
