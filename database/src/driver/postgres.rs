//! Postgres driver: query/execute/transaction/prepare.

use crate::driver::{
    ColumnMeta, ExecuteResult, Isolation, QueryResult, Row, TxStatement, TxStepResult,
};
use crate::error::DbError;
use crate::pool::PostgresPool;
use crate::value::{JsonParam, RowValue};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use postgres_types::{ToSql, Type};
use serde_json::Value as JsonValue;
use std::time::Duration;
use tokio_postgres::{Client, Statement};

pub async fn query(
    pool: &PostgresPool,
    sql: &str,
    params: &[JsonParam],
    timeout_ms: u64,
) -> Result<QueryResult, DbError> {
    let mut client = pool.acquire().await?;
    let bound = bind_params(params);
    let bound_refs: Vec<&(dyn ToSql + Sync)> =
        bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

    // `database::query` is the READ surface a narrowed agent policy grants;
    // a read-only transaction makes the server enforce it (writes fail with
    // 25006 read_only_sql_transaction) — this also catches data-modifying
    // CTEs (`WITH ... INSERT`) that keyword classification misses. Dropped
    // on timeout (implicit rollback); committed after rows are collected.
    let work = async {
        let tx = client
            .build_transaction()
            .read_only(true)
            .start()
            .await
            .map_err(map_err)?;
        let rows = tx
            .query(sql, bound_refs.as_slice())
            .await
            .map_err(map_err)?;
        tx.commit().await.map_err(map_err)?;
        Ok::<_, DbError>(rows)
    };
    let rows = tokio::time::timeout(Duration::from_millis(timeout_ms), work)
        .await
        .map_err(|_| DbError::QueryTimeout {
            db: "(pg)".into(),
            timeout_ms,
        })??;

    if rows.is_empty() {
        return Ok(QueryResult {
            columns: vec![],
            rows: vec![],
        });
    }

    let columns: Vec<ColumnMeta> = rows[0]
        .columns()
        .iter()
        .map(|c| ColumnMeta {
            name: c.name().to_string(),
            ty: c.type_().name().to_string(),
        })
        .collect();

    let mut out_rows: Vec<Row> = Vec::with_capacity(rows.len());
    for row in rows {
        let mut cells = Vec::with_capacity(row.columns().len());
        for (i, col) in row.columns().iter().enumerate() {
            cells.push(pg_cell_to_row_value(&row, i, col.type_())?);
        }
        out_rows.push(Row(cells));
    }
    Ok(QueryResult {
        columns,
        rows: out_rows,
    })
}

fn bind_params(params: &[JsonParam]) -> Vec<PgBind> {
    params.iter().map(PgBind::from_param).collect()
}

#[derive(Debug)]
enum PgBind {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Json(JsonValue),
}

impl PgBind {
    fn from_param(p: &JsonParam) -> Self {
        match p {
            JsonParam::Null => PgBind::Null,
            JsonParam::Bool(b) => PgBind::Bool(*b),
            JsonParam::Int(i) => PgBind::Int(*i),
            JsonParam::Float(f) => PgBind::Float(*f),
            JsonParam::Text(s) => PgBind::Text(s.clone()),
            JsonParam::Json(v) => PgBind::Json(v.clone()),
        }
    }
}

impl ToSql for PgBind {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        // Postgres binary protocol requires the wire-format byte width to
        // match the column's declared type. JsonParam carries i64/f64 but
        // columns are commonly INT4 / FLOAT4 / etc. Without dispatching on
        // `ty`, an `i64.to_sql(INT4, ...)` writes 8 bytes where the server
        // expects 4, producing SQLSTATE 22P03 (invalid_binary_representation).
        // Coerce numeric variants to the column's actual type before binding.
        match self {
            PgBind::Null => Ok(postgres_types::IsNull::Yes),
            PgBind::Bool(b) => b.to_sql(ty, out),
            PgBind::Int(i) => match *ty {
                // Reject overflow rather than silently wrapping. Without
                // try_from, `(*i as i16)` truncates 40000 → -25536 and writes
                // it to the column; that's silent data corruption with no
                // server-side error since the wire bytes are technically valid.
                Type::INT2 => i16::try_from(*i)
                    .map_err(|_| format!("value {i} out of range for INT2 (i16)").into())
                    .and_then(|v: i16| v.to_sql(ty, out)),
                Type::INT4 => i32::try_from(*i)
                    .map_err(|_| format!("value {i} out of range for INT4 (i32)").into())
                    .and_then(|v: i32| v.to_sql(ty, out)),
                Type::INT8 => i.to_sql(ty, out),
                Type::FLOAT4 => (*i as f32).to_sql(ty, out),
                Type::FLOAT8 => (*i as f64).to_sql(ty, out),
                _ => i.to_sql(ty, out),
            },
            PgBind::Float(f) => match *ty {
                Type::FLOAT4 => (*f as f32).to_sql(ty, out),
                _ => f.to_sql(ty, out),
            },
            PgBind::Text(s) => s.to_sql(ty, out),
            PgBind::Json(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    postgres_types::to_sql_checked!();
}

fn pg_cell_to_row_value(
    row: &tokio_postgres::Row,
    idx: usize,
    ty: &Type,
) -> Result<RowValue, DbError> {
    macro_rules! get {
        ($t:ty) => {{
            let v: Option<$t> = row.try_get(idx).map_err(map_err)?;
            v
        }};
    }
    use tokio_postgres::types::Type as T;
    Ok(match *ty {
        T::BOOL => match get!(bool) {
            Some(b) => RowValue::Bool(b),
            None => RowValue::Null,
        },
        T::INT2 => match get!(i16) {
            Some(i) => RowValue::Int(i as i64),
            None => RowValue::Null,
        },
        T::INT4 => match get!(i32) {
            Some(i) => RowValue::Int(i as i64),
            None => RowValue::Null,
        },
        T::INT8 => match get!(i64) {
            Some(i) => RowValue::BigInt(i),
            None => RowValue::Null,
        },
        T::FLOAT4 => match get!(f32) {
            Some(f) => RowValue::Float(f as f64),
            None => RowValue::Null,
        },
        T::FLOAT8 => match get!(f64) {
            Some(f) => RowValue::Float(f),
            None => RowValue::Null,
        },
        T::TEXT | T::VARCHAR | T::BPCHAR | T::NAME => match get!(String) {
            Some(s) => RowValue::Text(s),
            None => RowValue::Null,
        },
        // `String: FromSql` accepts only the text-family OIDs
        // (postgres-types-0.2/src/lib.rs:729), so decoding a uuid cell as
        // String failed with WrongType and took the whole query down — the
        // same class as the `"char"` and NUMERIC arms in this function.
        // Decode as Uuid and surface the canonical lowercase-hyphenated text.
        T::UUID => match get!(uuid::Uuid) {
            Some(u) => RowValue::Text(u.to_string()),
            None => RowValue::Null,
        },
        // The internal one-byte "char" (OID 18) — what pg_catalog uses for
        // relkind/contype/attidentity. It is NOT text-family: `String`'s
        // FromSql rejects it (so the `_` fallback below fails with "error
        // deserializing column N"), and tokio-postgres decodes it as i8.
        T::CHAR => match get!(i8) {
            Some(c) => RowValue::Text(((c as u8) as char).to_string()),
            None => RowValue::Null,
        },
        T::BYTEA => match get!(Vec<u8>) {
            Some(b) => RowValue::Bytes(b),
            None => RowValue::Null,
        },
        // postgres-types' chrono FromSql impls bind by exact OID:
        // `DateTime<Utc>` declares `accepts!(TIMESTAMPTZ)` and `NaiveDateTime`
        // declares `accepts!(TIMESTAMP)`. Decoding TIMESTAMP (no tz) as
        // `DateTime<Utc>` fails at runtime with WrongType. Split the arms:
        // TIMESTAMP → NaiveDateTime, then assume UTC for the wire envelope
        // so RowValue::Timestamp keeps its DateTime<Utc> shape.
        T::TIMESTAMPTZ => match get!(DateTime<Utc>) {
            Some(t) => RowValue::Timestamp(t),
            None => RowValue::Null,
        },
        T::TIMESTAMP => match get!(NaiveDateTime) {
            Some(n) => RowValue::Timestamp(Utc.from_utc_datetime(&n)),
            None => RowValue::Null,
        },
        T::JSON | T::JSONB => match get!(JsonValue) {
            Some(v) => RowValue::Json(v),
            None => RowValue::Null,
        },
        T::NUMERIC => {
            // Layered decode:
            //   1. rust_decimal::Decimal — fast, well-tested, handles 99% of
            //      real-world NUMERIC values (96-bit / ~28 sig digits).
            //   2. Custom binary parser fallback — for values rust_decimal
            //      rejects: NaN, ±Infinity, and arbitrary-precision NUMERIC
            //      beyond 96 bits (rust_decimal-1.41/src/postgres/driver.rs:91
            //      returns `Err(ConversionTo)` for special signs and line 109
            //      returns `Err(ExceedsMaximumPossibleValue)` for overflow).
            //   3. Null on parse failure.
            //
            // The reviewer originally suggested `get!(String)` as the
            // fallback, but `String: FromSql::accepts` is gated to TEXT-
            // family OIDs (postgres-types-0.2/src/lib.rs:729) and rejects
            // NUMERIC at runtime — confirmed earlier in this branch. The
            // custom parser sidesteps that by accepting NUMERIC directly
            // and stringifying the binary digits.
            match row.try_get::<_, Option<rust_decimal::Decimal>>(idx) {
                Ok(Some(d)) => RowValue::Decimal(d.to_string()),
                Ok(None) => RowValue::Null,
                Err(e) => {
                    tracing::debug!(
                        column = idx,
                        error = %e,
                        "NUMERIC outside rust_decimal range; falling back to binary parser"
                    );
                    match row.try_get::<_, Option<PgNumericText>>(idx) {
                        Ok(Some(t)) => RowValue::Decimal(t.0),
                        Ok(None) => RowValue::Null,
                        Err(e) => {
                            tracing::warn!(
                                column = idx,
                                error = %e,
                                "NUMERIC binary parser also failed; surfacing Null"
                            );
                            RowValue::Null
                        }
                    }
                }
            }
        }
        _ => {
            // Unknown / unmapped type — fall back to text representation.
            match row.try_get::<_, Option<String>>(idx).map_err(map_err)? {
                Some(s) => RowValue::Text(s),
                None => RowValue::Null,
            }
        }
    })
}

pub(crate) fn map_err(e: tokio_postgres::Error) -> DbError {
    let code = e.code().map(|c| c.code().to_string());
    DbError::DriverError {
        driver: "postgres".into(),
        code,
        message: e.to_string(),
        failed_index: None,
    }
}

/// True when the SQL statement is an INSERT — the only statement whose first
/// `RETURNING` cell can honestly be reported as `last_insert_id`. Same naïve
/// prefix check as the sqlite and mysql drivers, with the same safe
/// false-negatives: `WITH … INSERT` and comment-prefixed INSERTs report `None`
/// rather than the first cell of whatever rows came back.
fn is_insert(sql: &str) -> bool {
    sql.trim_start().to_ascii_uppercase().starts_with("INSERT")
}

/// Stamp a transaction-step index onto an error raised by a helper with no
/// step context (`map_err`, `pg_cell_to_row_value`). An index already present
/// is kept. Mirrors `driver::sqlite::with_failed_index`.
fn with_failed_index(e: DbError, idx: usize) -> DbError {
    match e {
        DbError::DriverError {
            driver,
            code,
            message,
            failed_index,
        } => DbError::DriverError {
            driver,
            code,
            message,
            failed_index: failed_index.or(Some(idx)),
        },
        other => other,
    }
}

fn column_meta(columns: &[tokio_postgres::Column]) -> Vec<ColumnMeta> {
    columns
        .iter()
        .map(|c| ColumnMeta {
            name: c.name().to_string(),
            ty: c.type_().name().to_string(),
        })
        .collect()
}

fn row_cells(row: &tokio_postgres::Row) -> Result<Row, DbError> {
    let mut cells = Vec::with_capacity(row.columns().len());
    for (i, col) in row.columns().iter().enumerate() {
        cells.push(pg_cell_to_row_value(row, i, col.type_())?);
    }
    Ok(Row(cells))
}

/// Run one prepared write statement and shape its result.
///
/// The prepared statement's column list is the planner's answer to "does this
/// statement produce rows" — what the sqlite driver already does. It replaces
/// a `contains(" RETURNING ")` text check that missed the keyword on its own
/// line (the rows came back, `client.execute` discarded them, and the caller
/// saw `returned_rows: []` with no error — and a `row-changed` event with no
/// `returning`) and matched the word inside a literal or comment (routing a
/// plain write through the query path, which then reported `affected_rows: 0`
/// for a statement that had committed, so no event fired at all). The same
/// list routes `EXPLAIN`, `SELECT` and `VALUES` sent through `execute`, which
/// now return their rows instead of a bare count.
///
/// `affected_rows` on the row-returning path is the number of rows returned:
/// for `RETURNING` that IS the number of rows changed, and tokio-postgres does
/// not surface the command tag alongside the rows.
async fn run_write(
    client: &Client,
    stmt: &Statement,
    sql: &str,
    params: &[JsonParam],
) -> Result<ExecuteResult, DbError> {
    let bound = bind_params(params);
    let bound_refs: Vec<&(dyn ToSql + Sync)> =
        bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

    if stmt.columns().is_empty() {
        let n = client
            .execute(stmt, bound_refs.as_slice())
            .await
            .map_err(map_err)?;
        return Ok(ExecuteResult {
            affected_rows: n,
            last_insert_id: None,
            returned_rows: vec![],
            returned_columns: vec![],
        });
    }

    let rows = client
        .query(stmt, bound_refs.as_slice())
        .await
        .map_err(map_err)?;
    let mut returned: Vec<Row> = Vec::with_capacity(rows.len());
    for row in &rows {
        returned.push(row_cells(row)?);
    }

    // Postgres has no `last_insert_rowid()` equivalent; we extract
    // `last_insert_id` from the first cell of the first RETURNING row. This
    // means the caller's RETURNING clause column ORDER is part of the
    // contract: `RETURNING id, name` produces last_insert_id = the id column;
    // `RETURNING name, id` produces last_insert_id = the name column (which
    // is rarely useful). Convention for callers who want the row's PK as
    // last_insert_id: put it first in RETURNING.
    //
    // Gated on INSERT: every row-producing statement takes this path now, and
    // a `SELECT name, id FROM t` sent through `execute` must not report its
    // first cell as an insert id.
    let last_insert_id = if is_insert(sql) {
        returned
            .first()
            .and_then(|r| r.0.first())
            .and_then(|first| match first {
                RowValue::Int(i) => Some(i.to_string()),
                RowValue::BigInt(i) => Some(i.to_string()),
                RowValue::Text(s) => Some(s.clone()),
                _ => None,
            })
    } else {
        None
    };

    Ok(ExecuteResult {
        affected_rows: returned.len() as u64,
        last_insert_id,
        returned_rows: returned,
        returned_columns: column_meta(stmt.columns()),
    })
}

/// The `returning` option does nothing on postgres: it never adds a
/// RETURNING clause, and the rows come from the SQL alone. SQLite refuses the
/// contradiction, MySQL warns; postgres used to stay silent, which is how a
/// caller ends up believing the option is what produces the rows.
fn warn_returning_option_ignored(returning: &[String]) {
    if !returning.is_empty() {
        tracing::warn!(
            driver = "postgres",
            "the `returning` option does not add a RETURNING clause and is ignored; \
             write `RETURNING {}` into the SQL itself",
            returning.join(", ")
        );
    }
}

pub async fn execute(
    pool: &PostgresPool,
    sql: &str,
    params: &[JsonParam],
    returning: &[String],
) -> Result<ExecuteResult, DbError> {
    warn_returning_option_ignored(returning);
    let client = pool.acquire().await?;
    // Prepare explicitly — not deadpool's `prepare_cached`: a cached plan
    // outlives the call, and a later statement that recreates a table with
    // a different shape would hit `0A000 cached plan must not change result
    // type`. `client.query(&str)` prepares internally anyway, so this is one
    // round trip fewer, not one more.
    let stmt = client.prepare(sql).await.map_err(map_err)?;
    run_write(&client, &stmt, sql, params).await
}

pub async fn transaction(
    pool: &PostgresPool,
    statements: Vec<TxStatement>,
    isolation: Option<Isolation>,
) -> Result<Vec<TxStepResult>, DbError> {
    let client = pool.acquire().await?;
    let begin_sql = match isolation {
        Some(Isolation::ReadCommitted) => "BEGIN ISOLATION LEVEL READ COMMITTED",
        Some(Isolation::RepeatableRead) => "BEGIN ISOLATION LEVEL REPEATABLE READ",
        Some(Isolation::Serializable) => "BEGIN ISOLATION LEVEL SERIALIZABLE",
        None => "BEGIN",
    };
    client.batch_execute(begin_sql).await.map_err(map_err)?;

    let mut results: Vec<TxStepResult> = Vec::with_capacity(statements.len());
    for (idx, stmt) in statements.iter().enumerate() {
        // Every failure of a step — prepare, execute, cell conversion — takes
        // this one exit: ROLLBACK, then the error stamped with the step index.
        // Returning early past the ROLLBACK would hand a connection still
        // inside BEGIN back to the pool (the Fast recycler issues no ROLLBACK),
        // and every later caller drawing it would fail with "current
        // transaction is aborted, commands ignored".
        match run_tx_step(&client, stmt).await {
            Ok(step) => results.push(step),
            Err(e) => {
                let _ = client.batch_execute("ROLLBACK").await;
                return Err(with_failed_index(e, idx));
            }
        }
    }

    if let Err(e) = client.batch_execute("COMMIT").await {
        // Best-effort ROLLBACK so the connection isn't returned to the pool
        // mid-transaction. deadpool's Fast recycler does not issue ROLLBACK,
        // so without this the next caller on this connection sees
        // "current transaction is aborted, commands ignored".
        let _ = client.batch_execute("ROLLBACK").await;
        return Err(map_err(e));
    }
    Ok(results)
}

/// One step of an atomic batch. Prepared here, after the previous steps ran,
/// because a step may create the table the next one writes to. The `Statement`
/// is dropped with this frame, so its `Close` goes out before COMMIT.
async fn run_tx_step(client: &Client, stmt: &TxStatement) -> Result<TxStepResult, DbError> {
    let prepared = client.prepare(&stmt.sql).await.map_err(map_err)?;
    let result = run_write(client, &prepared, &stmt.sql, &stmt.params).await?;
    Ok(TxStepResult {
        affected_rows: result.affected_rows,
        rows: result.returned_rows,
        columns: result.returned_columns,
    })
}

/// Issue `BEGIN [ISOLATION LEVEL ...]` on a pinned client. Used by
/// `beginTransaction` to open an interactive transaction on a connection
/// pulled out of the pool.
pub async fn tx_begin(
    client: &mut crate::pool::postgres::PgClient,
    isolation: Option<Isolation>,
) -> Result<(), DbError> {
    let begin_sql = match isolation {
        Some(Isolation::ReadCommitted) => "BEGIN ISOLATION LEVEL READ COMMITTED",
        Some(Isolation::RepeatableRead) => "BEGIN ISOLATION LEVEL REPEATABLE READ",
        Some(Isolation::Serializable) => "BEGIN ISOLATION LEVEL SERIALIZABLE",
        None => "BEGIN",
    };
    client.batch_execute(begin_sql).await.map_err(map_err)
}

/// `COMMIT` the in-progress transaction on a pinned client.
pub async fn tx_commit(client: &mut crate::pool::postgres::PgClient) -> Result<(), DbError> {
    // PostgreSQL accepts COMMIT in an aborted transaction but reports the
    // command tag ROLLBACK; batch_execute discards that tag. Probe while the
    // transaction is still open so 25P02 takes the ordinary commit-error path.
    client.simple_query("SELECT 1").await.map_err(map_err)?;
    client.batch_execute("COMMIT").await.map_err(map_err)
}

/// `ROLLBACK` the in-progress transaction on a pinned client. Callers that
/// want best-effort rollback (timeout watcher, post-commit-failure cleanup)
/// should `let _ =` the result.
pub async fn tx_rollback(client: &mut crate::pool::postgres::PgClient) -> Result<(), DbError> {
    client.batch_execute("ROLLBACK").await.map_err(map_err)
}

/// Run an INSERT/UPDATE/DELETE (optionally with `RETURNING`) on a pinned
/// client that is currently inside `BEGIN ... COMMIT`. Mirrors `execute()`
/// — but without pool acquire. Used by `transactionExecute`. A failing
/// statement leaves the transaction aborted for the caller to finalize, as
/// before: the registry owns the connection, nothing here returns it to a pool.
pub async fn tx_execute(
    client: &mut crate::pool::postgres::PgClient,
    sql: &str,
    params: &[JsonParam],
    returning: &[String],
) -> Result<ExecuteResult, DbError> {
    warn_returning_option_ignored(returning);
    let stmt = client.prepare(sql).await.map_err(map_err)?;
    run_write(client, &stmt, sql, params).await
}

pub async fn run_prepared(
    client: &mut crate::pool::postgres::PgClient,
    sql: &str,
    params: &[JsonParam],
) -> Result<QueryResult, DbError> {
    let bound = bind_params(params);
    let bound_refs: Vec<&(dyn ToSql + Sync)> =
        bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
    let stmt = client.prepare(sql).await.map_err(map_err)?;
    let rows = client
        .query(&stmt, bound_refs.as_slice())
        .await
        .map_err(map_err)?;
    let columns: Vec<ColumnMeta> = stmt
        .columns()
        .iter()
        .map(|c| ColumnMeta {
            name: c.name().to_string(),
            ty: c.type_().name().to_string(),
        })
        .collect();
    let mut out_rows: Vec<Row> = Vec::with_capacity(rows.len());
    for row in rows {
        let mut cells = Vec::with_capacity(row.columns().len());
        for (i, col) in row.columns().iter().enumerate() {
            cells.push(pg_cell_to_row_value(&row, i, col.type_())?);
        }
        out_rows.push(Row(cells));
    }
    Ok(QueryResult {
        columns,
        rows: out_rows,
    })
}

/// Fallback NUMERIC decoder used when `rust_decimal` rejects a value
/// (NaN, ±Infinity, or precision beyond 96 bits). Captures the raw
/// Postgres binary numeric format and stringifies it directly so the
/// caller sees a precision-preserving decimal/special-value string.
struct PgNumericText(String);

impl<'a> postgres_types::FromSql<'a> for PgNumericText {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        stringify_pg_numeric_binary(raw)
            .map(PgNumericText)
            .map_err(|e| -> Box<dyn std::error::Error + Sync + Send> { e.into() })
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}

/// Postgres binary NUMERIC layout (network byte order, all 16-bit fields):
///
///   u16 ndigits     — number of base-10000 digit groups
///   i16 weight      — weight of first group (signed)
///   u16 sign        — 0x0000=+, 0x4000=−, 0xC000=NaN, 0xD000=+Inf, 0xF000=−Inf
///   u16 dscale      — display scale (decimal fractional digits)
///   u16 digits[]    — base-10000 digits, MSB first
///
/// References: rust_decimal-1.41/src/postgres/driver.rs (the impl we fall
/// back from, which uses the same layout) and the Postgres source at
/// `src/backend/utils/adt/numeric.c`. The custom stringifier handles every
/// shape rust_decimal rejects so callers see a value rather than an error.
fn stringify_pg_numeric_binary(raw: &[u8]) -> Result<String, &'static str> {
    if raw.len() < 8 {
        return Err("numeric: header too short");
    }
    let ndigits = u16::from_be_bytes([raw[0], raw[1]]) as usize;
    let weight = i16::from_be_bytes([raw[2], raw[3]]) as i32;
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let dscale = u16::from_be_bytes([raw[6], raw[7]]) as usize;

    match sign {
        0xC000 => return Ok("NaN".to_string()),
        0xD000 => return Ok("Infinity".to_string()),
        0xF000 => return Ok("-Infinity".to_string()),
        0x0000 | 0x4000 => {} // positive / negative — fall through
        _ => return Err("numeric: unknown sign"),
    }

    if raw.len() < 8 + ndigits * 2 {
        return Err("numeric: digit buffer truncated");
    }

    let mut digits: Vec<u16> = Vec::with_capacity(ndigits);
    for i in 0..ndigits {
        let off = 8 + i * 2;
        let d = u16::from_be_bytes([raw[off], raw[off + 1]]);
        if d >= 10000 {
            return Err("numeric: digit out of range");
        }
        digits.push(d);
    }

    // Integer part. Empty digits or weight < 0 → "0".
    let mut int_part = String::new();
    if ndigits == 0 || weight < 0 {
        int_part.push('0');
    } else {
        let weight_u = weight as usize;
        let stored_int_count = std::cmp::min(ndigits, weight_u + 1);
        for (i, &d) in digits.iter().take(stored_int_count).enumerate() {
            if i == 0 {
                int_part.push_str(&d.to_string());
            } else {
                int_part.push_str(&format!("{d:04}"));
            }
        }
        // Append zero groups for integer positions beyond stored digits.
        let trailing_zero_groups = (weight_u + 1).saturating_sub(stored_int_count);
        for _ in 0..trailing_zero_groups {
            int_part.push_str("0000");
        }
    }

    // Fractional part. Only emit if dscale > 0.
    let mut frac_part = String::new();
    if dscale > 0 {
        // Leading zero groups when weight < -1 (the value is < 0.0001).
        if weight < -1 {
            let leading = ((-weight) - 1) as usize;
            for _ in 0..leading {
                frac_part.push_str("0000");
            }
        }
        // Stored fractional digits start where integer digits ended.
        let frac_start_idx = if weight < 0 { 0 } else { (weight as usize) + 1 };
        for &d in digits.iter().skip(frac_start_idx) {
            frac_part.push_str(&format!("{d:04}"));
        }
        // Trim or right-pad to exactly dscale chars.
        if frac_part.len() > dscale {
            frac_part.truncate(dscale);
        }
        while frac_part.len() < dscale {
            frac_part.push('0');
        }
    }

    let mut out = String::new();
    if sign == 0x4000 {
        out.push('-');
    }
    out.push_str(&int_part);
    if !frac_part.is_empty() {
        out.push('.');
        out.push_str(&frac_part);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::pool::PostgresPool;
    use crate::value::{JsonParam, RowValue};

    fn url() -> Option<String> {
        std::env::var("TEST_POSTGRES_URL").ok()
    }

    async fn fresh_pool() -> Option<PostgresPool> {
        let u = url()?;
        let tls = crate::config::TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ..Default::default()
        };
        Some(
            PostgresPool::new(&u, &PoolConfig::default(), &tls)
                .await
                .unwrap(),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_returns_rows_with_int_text_bool_null() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(
            &p,
            "SELECT 1::int AS a, 'x'::text AS b, true AS c, NULL::int AS d",
            &[],
            30_000,
        )
        .await
        .unwrap();
        assert_eq!(r.columns.len(), 4);
        assert!(matches!(&r.rows[0].0[0], RowValue::Int(1)));
        assert!(matches!(&r.rows[0].0[1], RowValue::Text(s) if s == "x"));
        assert!(matches!(&r.rows[0].0[2], RowValue::Bool(true)));
        assert!(matches!(&r.rows[0].0[3], RowValue::Null));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_with_positional_params() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(
            &p,
            "SELECT $1::int + $2::int AS sum",
            &[JsonParam::Int(2), JsonParam::Int(3)],
            30_000,
        )
        .await
        .unwrap();
        assert!(matches!(&r.rows[0].0[0], RowValue::Int(5)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_jsonb_round_trips_as_value() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(&p, "SELECT '{\"k\":1}'::jsonb AS j", &[], 30_000)
            .await
            .unwrap();
        match &r.rows[0].0[0] {
            RowValue::Json(v) => assert_eq!(v["k"], 1),
            other => panic!("expected Json, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_bigint_returns_string() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(&p, "SELECT 9007199254740993::bigint AS big", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(
            &r.rows[0].0[0],
            RowValue::BigInt(9_007_199_254_740_993)
        ));
    }

    /// Regression: `String: FromSql::accepts` is gated to TEXT-family OIDs
    /// (postgres-types-0.2/src/lib.rs:729), so the previous
    /// `try_get::<_, Option<String>>` on a NUMERIC column failed at runtime
    /// with WrongType and the entire RPC call rejected. The driver now
    /// decodes via `rust_decimal::Decimal` (which declares
    /// `accepts!(NUMERIC)` under the `db-tokio-postgres` feature) and
    /// stringifies to keep RowValue::Decimal precision-preserving on the
    /// wire. This test pins both that the decode succeeds and that the
    /// stringified form matches the source literal.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_decodes_numeric_to_string() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(
            &p,
            "SELECT 12345.6789::numeric AS exact, \
                    -0.001::numeric          AS negf, \
                    0::numeric               AS zero",
            &[],
            30_000,
        )
        .await
        .unwrap();
        match &r.rows[0].0[0] {
            RowValue::Decimal(s) => assert_eq!(s, "12345.6789"),
            other => panic!("exact: expected Decimal, got {other:?}"),
        }
        match &r.rows[0].0[1] {
            RowValue::Decimal(s) => assert_eq!(s, "-0.001"),
            other => panic!("negf: expected Decimal, got {other:?}"),
        }
        // rust_decimal stringifies zero as "0" (no trailing decimals when
        // dscale=0); we just assert it's a Decimal variant carrying "0".
        match &r.rows[0].0[2] {
            RowValue::Decimal(s) => assert_eq!(s, "0"),
            other => panic!("zero: expected Decimal, got {other:?}"),
        }
    }

    /// Regression: integration test for the layered NUMERIC decode path
    /// against a live postgres. Exercises three values that previously
    /// failed the entire query because rust_decimal returned Err for them:
    ///   - 'NaN'::numeric            → rust_decimal Err(ConversionTo)
    ///   - very large numeric        → rust_decimal Err(ExceedsMaximumPossibleValue)
    ///   - 'Infinity'::numeric       → rust_decimal Err(ConversionTo)
    /// The fix routes these through PgNumericText, which decodes the
    /// Postgres binary format directly and surfaces a precision-preserving
    /// string. This test asserts the wiring: `try_get<rust_decimal>` errors,
    /// `try_get<PgNumericText>` succeeds, and the final RowValue carries
    /// the right string. Gated on TEST_POSTGRES_URL like the other pg tests.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_falls_back_to_binary_parser_for_numeric_edge_cases() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(
            &p,
            "SELECT 'NaN'::numeric                                AS nan, \
                    'Infinity'::numeric                            AS pinf, \
                    '-Infinity'::numeric                           AS ninf, \
                    100000000000000000000000000000::numeric        AS big",
            &[],
            30_000,
        )
        .await
        .unwrap();
        match &r.rows[0].0[0] {
            RowValue::Decimal(s) => assert_eq!(s, "NaN"),
            other => panic!("nan: expected Decimal(\"NaN\"), got {other:?}"),
        }
        match &r.rows[0].0[1] {
            RowValue::Decimal(s) => assert_eq!(s, "Infinity"),
            other => panic!("pinf: expected Decimal(\"Infinity\"), got {other:?}"),
        }
        match &r.rows[0].0[2] {
            RowValue::Decimal(s) => assert_eq!(s, "-Infinity"),
            other => panic!("ninf: expected Decimal(\"-Infinity\"), got {other:?}"),
        }
        // 10^29 — beyond rust_decimal's ~10^28 cap. Pre-fix this exploded the
        // entire query; now the binary parser stringifies it exactly.
        match &r.rows[0].0[3] {
            RowValue::Decimal(s) => {
                assert_eq!(s, "100000000000000000000000000000");
            }
            other => panic!("big: expected Decimal, got {other:?}"),
        }
    }

    /// Unit tests for the binary NUMERIC fallback parser. Drive it with
    /// crafted byte sequences in the documented Postgres format so we don't
    /// need a live postgres connection — the parser is the part of the fix
    /// that handles values rust_decimal rejects (NaN, ±Infinity, large
    /// magnitudes), and the production code path `try_get → from_sql` only
    /// exercises it when the primary decode errors.
    #[test]
    fn stringify_pg_numeric_handles_zero() {
        // ndigits=0, weight=0, sign=+, dscale=0 → "0"
        let raw = [0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(stringify_pg_numeric_binary(&raw).unwrap(), "0");
    }

    #[test]
    fn stringify_pg_numeric_handles_zero_with_scale() {
        // ndigits=0, weight=0, sign=+, dscale=5 → "0.00000"
        let raw = [0, 0, 0, 0, 0, 0, 0, 5];
        assert_eq!(stringify_pg_numeric_binary(&raw).unwrap(), "0.00000");
    }

    #[test]
    fn stringify_pg_numeric_handles_simple_fraction() {
        // 1234.5 → ndigits=2, weight=0, sign=+, dscale=1, digits=[1234, 5000]
        let raw = [
            0, 2, // ndigits
            0, 0, // weight
            0, 0, // sign +
            0, 1, // dscale
            0x04, 0xD2, // 1234
            0x13, 0x88, // 5000
        ];
        assert_eq!(stringify_pg_numeric_binary(&raw).unwrap(), "1234.5");
    }

    #[test]
    fn stringify_pg_numeric_handles_small_fraction() {
        // 0.001 → ndigits=1, weight=-1, sign=+, dscale=3, digits=[10]
        let raw = [
            0, 1, // ndigits
            0xFF, 0xFF, // weight = -1
            0, 0, // sign +
            0, 3, // dscale
            0, 10, // digit
        ];
        assert_eq!(stringify_pg_numeric_binary(&raw).unwrap(), "0.001");
    }

    #[test]
    fn stringify_pg_numeric_handles_negative() {
        // -42 → ndigits=1, weight=0, sign=0x4000, dscale=0, digits=[42]
        let raw = [
            0, 1, // ndigits
            0, 0, // weight
            0x40, 0x00, // sign -
            0, 0, // dscale
            0, 42, // digit
        ];
        assert_eq!(stringify_pg_numeric_binary(&raw).unwrap(), "-42");
    }

    #[test]
    fn stringify_pg_numeric_handles_large_value_beyond_rust_decimal() {
        // 10^30 = 100 * 10000^7. ndigits=1, weight=7, sign=+, dscale=0,
        // digits=[100]. This value overflows rust_decimal (96-bit cap ~10^28),
        // so before the fallback parser it failed query calls outright.
        let raw = [
            0, 1, // ndigits
            0, 7, // weight
            0, 0, // sign +
            0, 0, // dscale
            0, 100, // digit
        ];
        assert_eq!(
            stringify_pg_numeric_binary(&raw).unwrap(),
            "1000000000000000000000000000000"
        );
    }

    #[test]
    fn stringify_pg_numeric_handles_special_signs() {
        let nan = [0, 0, 0, 0, 0xC0, 0x00, 0, 0];
        assert_eq!(stringify_pg_numeric_binary(&nan).unwrap(), "NaN");

        let pinf = [0, 0, 0, 0, 0xD0, 0x00, 0, 0];
        assert_eq!(stringify_pg_numeric_binary(&pinf).unwrap(), "Infinity");

        let ninf = [0, 0, 0, 0, 0xF0, 0x00, 0, 0];
        assert_eq!(stringify_pg_numeric_binary(&ninf).unwrap(), "-Infinity");
    }

    #[test]
    fn stringify_pg_numeric_rejects_truncated_buffers() {
        // Header too short
        assert!(stringify_pg_numeric_binary(&[0, 0]).is_err());
        // Header claims 3 digits but buffer is too small
        let raw = [0, 3, 0, 0, 0, 0, 0, 0, 0, 1];
        assert!(stringify_pg_numeric_binary(&raw).is_err());
    }

    /// Regression: `DateTime<Utc>: FromSql` declares `accepts!(TIMESTAMPTZ)`
    /// (postgres-types-0.2/src/chrono_04.rs:48), so decoding a TIMESTAMP (no
    /// tz) column as `DateTime<Utc>` fails at runtime with WrongType. The
    /// driver now decodes TIMESTAMP via `NaiveDateTime` and folds it into
    /// `RowValue::Timestamp(DateTime<Utc>)` by treating the naive value as
    /// UTC. This test pins both the failing-before path (TIMESTAMP) and the
    /// working path (TIMESTAMPTZ) so a regression on either side fails fast.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_decodes_timestamp_without_tz_and_with_tz() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(
            &p,
            "SELECT \
                '2026-04-29 12:00:00'::timestamp        AS naive, \
                '2026-04-29 12:00:00+00'::timestamptz   AS with_tz",
            &[],
            30_000,
        )
        .await
        .unwrap();
        // Both columns surface as RowValue::Timestamp; both round-trip through
        // RFC 3339 UTC at the wire. The buggy code panicked on the `naive`
        // column with a WrongType error before reaching this assertion.
        match &r.rows[0].0[0] {
            RowValue::Timestamp(t) => {
                assert_eq!(t.to_rfc3339(), "2026-04-29T12:00:00+00:00");
            }
            other => panic!("expected Timestamp for TIMESTAMP column, got {other:?}"),
        }
        match &r.rows[0].0[1] {
            RowValue::Timestamp(t) => {
                assert_eq!(t.to_rfc3339(), "2026-04-29T12:00:00+00:00");
            }
            other => panic!("expected Timestamp for TIMESTAMPTZ column, got {other:?}"),
        }
    }

    /// Regression: uuid shared the text arm and was decoded as `String`,
    /// whose `FromSql::accepts` is gated to text-family OIDs — every result
    /// with a uuid column failed with WrongType. The e2e fixtures key on
    /// BIGSERIAL, so nothing exercised it until the native-capture key work.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_query_decodes_uuid_as_text() {
        let Some(p) = fresh_pool().await else { return };
        let r = query(
            &p,
            "SELECT '550E8400-E29B-41D4-A716-446655440000'::uuid AS u, NULL::uuid AS none",
            &[],
            30_000,
        )
        .await
        .unwrap();
        match &r.rows[0].0[0] {
            RowValue::Text(s) => assert_eq!(s, "550e8400-e29b-41d4-a716-446655440000"),
            other => panic!("expected canonical uuid text, got {other:?}"),
        }
        assert!(matches!(&r.rows[0].0[1], RowValue::Null));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_execute_insert_returns_affected_count() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_t", &[], &[]).await;
        let _ = execute(
            &p,
            "CREATE TABLE db_w_t (id SERIAL PRIMARY KEY, n INT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        let r = execute(
            &p,
            "INSERT INTO db_w_t (n) VALUES ($1), ($2)",
            &[JsonParam::Int(1), JsonParam::Int(2)],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(r.affected_rows, 2);
        assert!(r.last_insert_id.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_execute_with_returning_populates_rows_and_last_insert_id() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_t2", &[], &[]).await;
        let _ = execute(
            &p,
            "CREATE TABLE db_w_t2 (id SERIAL PRIMARY KEY, n INT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        let r = execute(
            &p,
            "INSERT INTO db_w_t2 (n) VALUES ($1) RETURNING id, n",
            &[JsonParam::Int(7)],
            &["id".into(), "n".into()],
        )
        .await
        .unwrap();
        assert_eq!(r.returned_rows.len(), 1);
        assert_eq!(r.returned_columns.len(), 2);
        assert!(r.last_insert_id.is_some());
    }

    async fn pool_of_one() -> Option<PostgresPool> {
        let u = url()?;
        let tls = crate::config::TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ..Default::default()
        };
        let cfg = PoolConfig {
            max: 1,
            ..PoolConfig::default()
        };
        Some(PostgresPool::new(&u, &cfg, &tls).await.unwrap())
    }

    /// Regression: RETURNING was detected with `contains(" RETURNING ")`, so
    /// the keyword on its own line routed the statement through
    /// `client.execute`, which discards the rows — the caller saw
    /// `returned_rows: []` with no error, and the row-changed event carried
    /// no `returning`. The prepared statement's column list is the routing
    /// signal now, whatever the whitespace or comments around the keyword.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_execute_returning_split_across_lines_still_returns_rows() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_ml", &[], &[]).await;
        execute(
            &p,
            "CREATE TABLE db_w_ml (id SERIAL PRIMARY KEY, n INT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        for sql in [
            "INSERT INTO db_w_ml (n) VALUES ($1)\nRETURNING\nid, n",
            "INSERT INTO db_w_ml (n) VALUES ($1)\tRETURNING\tid, n",
            "INSERT INTO db_w_ml (n) VALUES ($1) RETURNING/*key*/id, n",
        ] {
            // No `returning` option: the SQL alone produces the rows.
            let r = execute(&p, sql, &[JsonParam::Int(7)], &[]).await.unwrap();
            assert_eq!(r.affected_rows, 1, "{sql}");
            assert_eq!(r.returned_rows.len(), 1, "{sql}");
            assert_eq!(r.returned_columns.len(), 2, "{sql}");
            assert_eq!(r.returned_columns[0].name, "id", "{sql}");
            assert!(r.last_insert_id.is_some(), "{sql}");
        }
        // Exactly three rows landed — each statement ran once.
        let q = query(&p, "SELECT COUNT(*) FROM db_w_ml", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(&q.rows[0].0[0], RowValue::BigInt(3)));
    }

    /// The inverse false positive: the keyword inside a literal or comment
    /// used to route a plain write through the query path, which reported
    /// `affected_rows: 0` for a statement that had committed — and the bus
    /// fires nothing for zero rows. Now it is a plain write with a real count.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_execute_keyword_in_literal_or_comment_is_a_plain_write() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_lit", &[], &[]).await;
        execute(
            &p,
            "CREATE TABLE db_w_lit (id SERIAL PRIMARY KEY, note TEXT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        for sql in [
            "INSERT INTO db_w_lit (note) VALUES ('please RETURNING soon')",
            "INSERT INTO db_w_lit (note) VALUES ('x') /* RETURNING id */",
            "INSERT INTO db_w_lit (note) VALUES ('y') -- RETURNING id\n",
        ] {
            let r = execute(&p, sql, &[], &[]).await.unwrap();
            assert_eq!(r.affected_rows, 1, "{sql}");
            assert!(r.returned_rows.is_empty(), "{sql}");
            assert!(r.last_insert_id.is_none(), "{sql}");
        }
    }

    /// Every row-producing statement takes the query path now, so a SELECT
    /// or VALUES sent through `execute` returns its rows (sqlite parity) —
    /// and must NOT report its first cell as an insert id.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_execute_select_returns_rows_without_last_insert_id() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_sel", &[], &[]).await;
        execute(
            &p,
            "CREATE TABLE db_w_sel (id SERIAL PRIMARY KEY, name TEXT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        execute(
            &p,
            "INSERT INTO db_w_sel (name) VALUES ('ann'), ('bob')",
            &[],
            &[],
        )
        .await
        .unwrap();

        let r = execute(&p, "SELECT name, id FROM db_w_sel ORDER BY id", &[], &[])
            .await
            .unwrap();
        assert_eq!(r.affected_rows, 2);
        assert_eq!(r.returned_rows.len(), 2);
        assert!(matches!(&r.returned_rows[0].0[0], RowValue::Text(s) if s == "ann"));
        assert!(
            r.last_insert_id.is_none(),
            "a SELECT's first cell is not an insert id: {:?}",
            r.last_insert_id
        );

        let r = execute(&p, "VALUES (10), (20), (30)", &[], &[])
            .await
            .unwrap();
        assert_eq!(r.affected_rows, 3);
        assert_eq!(r.returned_rows.len(), 3);
        assert!(r.last_insert_id.is_none());
    }

    /// Batch parity with `driver::sqlite`: CTE-prefixed SELECT, VALUES,
    /// comment-prefixed SELECT and multi-line RETURNING all populate
    /// `results[].rows`; a statement that returns nothing keeps its count.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_transaction_handles_cte_select_values_and_multiline_returning() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_shapes", &[], &[]).await;
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_shapes_into", &[], &[]).await;
        let stmts = vec![
            // A step may create the table the next one writes to: prepare
            // per step, after the previous ones ran.
            TxStatement {
                sql: "CREATE TABLE db_w_shapes (id SERIAL PRIMARY KEY, n INT)".into(),
                params: vec![],
            },
            TxStatement {
                sql: "WITH cte AS (SELECT 1 AS n) SELECT n FROM cte".into(),
                params: vec![],
            },
            TxStatement {
                sql: "VALUES (10), (20), (30)".into(),
                params: vec![],
            },
            TxStatement {
                sql: "INSERT INTO db_w_shapes (n) VALUES ($1)\nRETURNING\nid, n".into(),
                params: vec![JsonParam::Int(42)],
            },
            TxStatement {
                sql: "/* read */ SELECT n FROM db_w_shapes".into(),
                params: vec![],
            },
            TxStatement {
                sql: "UPDATE db_w_shapes SET n = n + 1 WHERE id = $1".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "SELECT n INTO db_w_shapes_into FROM db_w_shapes".into(),
                params: vec![],
            },
        ];
        let results = transaction(&p, stmts, None).await.unwrap();
        assert_eq!(results.len(), 7);
        assert_eq!(results[0].affected_rows, 0);
        assert!(results[0].rows.is_empty());
        assert_eq!(results[1].rows.len(), 1);
        assert_eq!(results[1].columns[0].name, "n");
        assert_eq!(results[2].rows.len(), 3);
        assert_eq!(results[3].affected_rows, 1);
        assert_eq!(results[3].rows.len(), 1);
        assert_eq!(results[3].columns.len(), 2);
        assert_eq!(results[4].rows.len(), 1);
        assert!(matches!(&results[4].rows[0].0[0], RowValue::Int(42)));
        assert_eq!(results[5].affected_rows, 1);
        assert!(results[5].rows.is_empty());
        // SELECT INTO produces no result columns: it is a write of one row.
        assert_eq!(results[6].affected_rows, 1);
        assert!(results[6].rows.is_empty());

        let _ = execute(&p, "DROP TABLE db_w_shapes_into", &[], &[]).await;
        let _ = execute(&p, "DROP TABLE db_w_shapes", &[], &[]).await;
    }

    /// Regression: a cell-conversion failure inside the batch loop used to
    /// `?` straight out of `transaction()`, past the ROLLBACK, handing a
    /// connection still inside BEGIN back to the pool — with `max: 1` every
    /// later caller drew that poisoned connection. Prepare failures now sit
    /// in the same position. Both must roll back, stamp the step index, and
    /// leave the connection reusable.
    #[tokio::test(flavor = "multi_thread")]
    async fn pg_transaction_step_failures_roll_back_and_free_the_connection() {
        let Some(p) = pool_of_one().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_poison", &[], &[]).await;
        execute(&p, "CREATE TABLE db_w_poison (n INT)", &[], &[])
            .await
            .unwrap();

        // Conversion failure: an int[] cell has no RowValue mapping and the
        // text fallback rejects the OID, so decoding errors after the
        // statement ran.
        let err = transaction(
            &p,
            vec![
                TxStatement {
                    sql: "INSERT INTO db_w_poison VALUES (1)".into(),
                    params: vec![],
                },
                TxStatement {
                    sql: "SELECT ARRAY[1, 2] AS cells".into(),
                    params: vec![],
                },
            ],
            None,
        )
        .await
        .unwrap_err();
        match &err {
            DbError::DriverError { failed_index, .. } => assert_eq!(*failed_index, Some(1)),
            other => panic!("expected DriverError, got {other:?}"),
        }

        // Prepare failure: the relation does not exist.
        let err = transaction(
            &p,
            vec![
                TxStatement {
                    sql: "INSERT INTO db_w_poison VALUES (2)".into(),
                    params: vec![],
                },
                TxStatement {
                    sql: "INSERT INTO db_w_poison_missing VALUES (2)".into(),
                    params: vec![],
                },
            ],
            None,
        )
        .await
        .unwrap_err();
        match &err {
            DbError::DriverError {
                failed_index, code, ..
            } => {
                assert_eq!(*failed_index, Some(1));
                assert_eq!(code.as_deref(), Some("42P01"), "{err}");
            }
            other => panic!("expected DriverError, got {other:?}"),
        }

        // The single pooled connection is not stuck in an aborted
        // transaction, and both first steps were rolled back.
        let q = query(&p, "SELECT COUNT(*) FROM db_w_poison", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(&q.rows[0].0[0], RowValue::BigInt(0)), "{q:?}");
        let _ = execute(&p, "DROP TABLE db_w_poison", &[], &[]).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_tx_execute_returning_split_across_lines_returns_rows() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_tx_ml", &[], &[]).await;
        execute(
            &p,
            "CREATE TABLE db_w_tx_ml (id SERIAL PRIMARY KEY, n INT)",
            &[],
            &[],
        )
        .await
        .unwrap();

        let mut client = p.acquire().await.unwrap();
        tx_begin(&mut client, None).await.unwrap();
        let r = tx_execute(
            &mut client,
            "INSERT INTO db_w_tx_ml (n) VALUES ($1)\nRETURNING id, n",
            &[JsonParam::Int(5)],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(r.affected_rows, 1);
        assert_eq!(r.returned_rows.len(), 1);
        assert!(r.last_insert_id.is_some());
        tx_commit(&mut client).await.unwrap();
        drop(client);
        let _ = execute(&p, "DROP TABLE db_w_tx_ml", &[], &[]).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_transaction_commits() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_tx", &[], &[]).await;
        let _ = execute(&p, "CREATE TABLE db_w_tx (n INT)", &[], &[])
            .await
            .unwrap();
        let stmts = vec![
            TxStatement {
                sql: "INSERT INTO db_w_tx VALUES ($1)".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "INSERT INTO db_w_tx VALUES ($1)".into(),
                params: vec![JsonParam::Int(2)],
            },
        ];
        let res = transaction(&p, stmts, Some(Isolation::ReadCommitted))
            .await
            .unwrap();
        assert_eq!(res.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_transaction_rolls_back_on_failure() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_tx2", &[], &[]).await;
        let _ = execute(&p, "CREATE TABLE db_w_tx2 (n INT NOT NULL)", &[], &[])
            .await
            .unwrap();
        let stmts = vec![
            TxStatement {
                sql: "INSERT INTO db_w_tx2 VALUES ($1)".into(),
                params: vec![JsonParam::Int(1)],
            },
            TxStatement {
                sql: "INSERT INTO db_w_tx2 VALUES ($1)".into(),
                params: vec![JsonParam::Null],
            },
        ];
        let err = transaction(&p, stmts, None).await.unwrap_err();
        assert!(matches!(err, DbError::DriverError { .. }));
        let r = query(&p, "SELECT COUNT(*) FROM db_w_tx2", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(&r.rows[0].0[0], RowValue::BigInt(0)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_interactive_commit_rejects_an_aborted_transaction() {
        let Some(p) = fresh_pool().await else { return };
        let _ = execute(&p, "DROP TABLE IF EXISTS db_w_aborted_tx", &[], &[]).await;
        execute(
            &p,
            "CREATE TABLE db_w_aborted_tx (n INT NOT NULL)",
            &[],
            &[],
        )
        .await
        .unwrap();

        let mut client = p.acquire().await.unwrap();
        tx_begin(&mut client, None).await.unwrap();
        tx_execute(
            &mut client,
            "INSERT INTO db_w_aborted_tx VALUES (1)",
            &[],
            &[],
        )
        .await
        .unwrap();
        tx_execute(
            &mut client,
            "INSERT INTO db_w_aborted_tx VALUES (NULL)",
            &[],
            &[],
        )
        .await
        .unwrap_err();

        assert!(tx_commit(&mut client).await.is_err());
        tx_rollback(&mut client).await.unwrap();
        drop(client);

        let rows = query(&p, "SELECT COUNT(*) FROM db_w_aborted_tx", &[], 30_000)
            .await
            .unwrap();
        assert!(matches!(&rows.rows[0].0[0], RowValue::BigInt(0)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_run_prepared_executes_with_params() {
        let Some(p) = fresh_pool().await else { return };
        let mut client = p.acquire().await.unwrap();
        let result = run_prepared(
            &mut client,
            "SELECT $1::int + $2::int AS total",
            &[JsonParam::Int(40), JsonParam::Int(2)],
        )
        .await
        .unwrap();
        assert!(matches!(&result.rows[0].0[0], RowValue::Int(42)));
    }
}
