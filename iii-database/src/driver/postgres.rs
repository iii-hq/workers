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

pub async fn query(
    pool: &PostgresPool,
    sql: &str,
    params: &[JsonParam],
    timeout_ms: u64,
) -> Result<QueryResult, DbError> {
    let client = pool.acquire().await?;
    let bound = bind_params(params);
    let bound_refs: Vec<&(dyn ToSql + Sync)> =
        bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

    let fut = client.query(sql, bound_refs.as_slice());
    let rows = tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
        .await
        .map_err(|_| DbError::QueryTimeout {
            db: "(pg)".into(),
            timeout_ms,
        })?
        .map_err(map_err)?;

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
        T::TEXT | T::VARCHAR | T::BPCHAR | T::NAME | T::UUID => match get!(String) {
            Some(s) => RowValue::Text(s),
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

pub async fn execute(
    pool: &PostgresPool,
    sql: &str,
    params: &[JsonParam],
    _returning: &[String],
) -> Result<ExecuteResult, DbError> {
    let client = pool.acquire().await?;
    let bound = bind_params(params);
    let bound_refs: Vec<&(dyn ToSql + Sync)> =
        bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

    let upper = sql.to_ascii_uppercase();
    if upper.contains(" RETURNING ") {
        let rows = client
            .query(sql, bound_refs.as_slice())
            .await
            .map_err(map_err)?;
        let columns: Vec<ColumnMeta> = rows
            .first()
            .map(|r| {
                r.columns()
                    .iter()
                    .map(|c| ColumnMeta {
                        name: c.name().to_string(),
                        ty: c.type_().name().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut returned: Vec<Row> = Vec::with_capacity(rows.len());
        let mut last_insert_id: Option<String> = None;

        // Postgres has no `last_insert_rowid()` equivalent; we extract
        // `last_insert_id` from the first cell of the first RETURNING row.
        // This means the caller's RETURNING clause column ORDER is part of
        // the contract: `RETURNING id, name` produces last_insert_id = the
        // id column; `RETURNING name, id` produces last_insert_id = the
        // name column (which is rarely useful).
        //
        // Convention for callers who want the row's PK as last_insert_id:
        // put it first in RETURNING.
        for (ri, row) in rows.iter().enumerate() {
            let mut cells = Vec::with_capacity(row.columns().len());
            for (i, col) in row.columns().iter().enumerate() {
                cells.push(pg_cell_to_row_value(row, i, col.type_())?);
            }
            if ri == 0 {
                if let Some(first) = cells.first() {
                    last_insert_id = match first {
                        RowValue::Int(i) => Some(i.to_string()),
                        RowValue::BigInt(i) => Some(i.to_string()),
                        RowValue::Text(s) => Some(s.clone()),
                        _ => None,
                    };
                }
            }
            returned.push(Row(cells));
        }

        Ok(ExecuteResult {
            affected_rows: returned.len() as u64,
            last_insert_id,
            returned_rows: returned,
            returned_columns: columns,
        })
    } else {
        let n = client
            .execute(sql, bound_refs.as_slice())
            .await
            .map_err(map_err)?;
        Ok(ExecuteResult {
            affected_rows: n,
            last_insert_id: None,
            returned_rows: vec![],
            returned_columns: vec![],
        })
    }
}

pub async fn transaction(
    pool: &PostgresPool,
    statements: Vec<TxStatement>,
    isolation: Option<Isolation>,
) -> Result<Vec<TxStepResult>, DbError> {
    let mut client = pool.acquire().await?;
    let begin_sql = match isolation {
        Some(Isolation::ReadCommitted) => "BEGIN ISOLATION LEVEL READ COMMITTED",
        Some(Isolation::RepeatableRead) => "BEGIN ISOLATION LEVEL REPEATABLE READ",
        Some(Isolation::Serializable) => "BEGIN ISOLATION LEVEL SERIALIZABLE",
        None => "BEGIN",
    };
    let tx_client = &mut *client;
    tx_client.batch_execute(begin_sql).await.map_err(map_err)?;

    let mut results: Vec<TxStepResult> = Vec::with_capacity(statements.len());

    for (idx, stmt) in statements.iter().enumerate() {
        let bound = bind_params(&stmt.params);
        let bound_refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let upper = stmt.sql.to_ascii_uppercase();
        let returns_rows =
            upper.trim_start().starts_with("SELECT") || upper.contains(" RETURNING ");

        let step = if returns_rows {
            match tx_client.query(&stmt.sql, bound_refs.as_slice()).await {
                Ok(rows) => {
                    let mut cells_rows: Vec<Row> = Vec::with_capacity(rows.len());
                    for row in &rows {
                        let mut cells = Vec::with_capacity(row.columns().len());
                        for (i, col) in row.columns().iter().enumerate() {
                            cells.push(pg_cell_to_row_value(row, i, col.type_())?);
                        }
                        cells_rows.push(Row(cells));
                    }
                    TxStepResult {
                        affected_rows: cells_rows.len() as u64,
                        rows: cells_rows,
                    }
                }
                Err(e) => {
                    let _ = tx_client.batch_execute("ROLLBACK").await;
                    return Err(step_err(idx, e));
                }
            }
        } else {
            match tx_client.execute(&stmt.sql, bound_refs.as_slice()).await {
                Ok(n) => TxStepResult {
                    affected_rows: n,
                    rows: vec![],
                },
                Err(e) => {
                    let _ = tx_client.batch_execute("ROLLBACK").await;
                    return Err(step_err(idx, e));
                }
            }
        };
        results.push(step);
    }

    if let Err(e) = tx_client.batch_execute("COMMIT").await {
        // Best-effort ROLLBACK so the connection isn't returned to the pool
        // mid-transaction. deadpool's Fast recycler does not issue ROLLBACK,
        // so without this the next caller on this connection sees
        // "current transaction is aborted, commands ignored".
        let _ = tx_client.batch_execute("ROLLBACK").await;
        return Err(map_err(e));
    }
    Ok(results)
}

fn step_err(idx: usize, e: tokio_postgres::Error) -> DbError {
    let code = e.code().map(|c| c.code().to_string());
    DbError::DriverError {
        driver: "postgres".into(),
        code,
        message: e.to_string(),
        failed_index: Some(idx),
    }
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
            ca_cert: None,
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
