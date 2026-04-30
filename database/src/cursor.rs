//! __iii_cursors table CRUD.
//!
//! This table tracks the last successfully-acked cursor for each query-poll
//! trigger. It is created on first poll inside the watched database.

use crate::driver::{self};
use crate::error::DbError;
use crate::pool::Pool;
use crate::value::JsonParam;

/// Default table name; overridable per-trigger via `cursor_table`.
pub const DEFAULT_CURSOR_TABLE: &str = "__iii_cursors";

/// Per-driver quoted form of the `cursor` column. `cursor` is a reserved
/// word in MySQL 8 (and standard SQL/PSM); without quoting, the table DDL
/// fails with ERROR 1064 syntax error. Postgres accepts unquoted but we
/// quote defensively for parity. SQLite needs no quoting but tolerates it.
fn cursor_col(pool: &Pool) -> &'static str {
    match pool {
        Pool::Postgres(_) => "\"cursor\"",
        Pool::Mysql(_) => "`cursor`",
        Pool::Sqlite(_) => "cursor",
    }
}

/// Issue `CREATE TABLE IF NOT EXISTS` for the cursor table. Called on every
/// poll tick. Intentionally NOT cached: a process-wide cache silently lies
/// when the table is dropped externally (e.g. test harness SCHEMA_RESET,
/// operator running `DROP TABLE __iii_cursors` for any reason), causing
/// every subsequent tick on the same worker process to silently fail
/// because read_cursor hits the missing table while the cache says
/// "ensured". `CREATE TABLE IF NOT EXISTS` against an existing table is a
/// system-catalog check on every driver — measured at sub-millisecond on
/// Postgres/MySQL/SQLite. At the default 1s polling interval that overhead
/// is invisible; the cache traded too much correctness for too little
/// throughput.
pub async fn ensure_table(pool: &Pool, table: &str) -> Result<(), DbError> {
    let cursor = cursor_col(pool);
    let sql = match pool {
        Pool::Postgres(_) => format!(
            "CREATE TABLE IF NOT EXISTS {table} (\
              trigger_id TEXT PRIMARY KEY, \
              {cursor} TEXT, \
              updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW())"
        ),
        Pool::Mysql(_) => format!(
            "CREATE TABLE IF NOT EXISTS {table} (\
              trigger_id VARCHAR(191) PRIMARY KEY, \
              {cursor} TEXT, \
              updated_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6))"
        ),
        Pool::Sqlite(_) => format!(
            "CREATE TABLE IF NOT EXISTS {table} (\
              trigger_id TEXT PRIMARY KEY, \
              {cursor} TEXT, \
              updated_at TEXT NOT NULL)"
        ),
    };
    match pool {
        Pool::Postgres(p) => driver::postgres::execute(p, &sql, &[], &[])
            .await
            .map(|_| ()),
        Pool::Mysql(p) => driver::mysql::execute(p, &sql, &[], &[]).await.map(|_| ()),
        Pool::Sqlite(p) => driver::sqlite::execute(p, &sql, &[], &[])
            .await
            .map(|_| ()),
    }
}

pub async fn read_cursor(
    pool: &Pool,
    table: &str,
    trigger_id: &str,
) -> Result<Option<String>, DbError> {
    let placeholder = match pool {
        Pool::Postgres(_) => "$1",
        _ => "?",
    };
    let cursor = cursor_col(pool);
    let sql = format!("SELECT {cursor} FROM {table} WHERE trigger_id = {placeholder} LIMIT 1");
    let result = match pool {
        Pool::Postgres(p) => {
            driver::postgres::query(p, &sql, &[JsonParam::Text(trigger_id.into())], 30_000).await?
        }
        Pool::Mysql(p) => {
            driver::mysql::query(p, &sql, &[JsonParam::Text(trigger_id.into())], 30_000).await?
        }
        Pool::Sqlite(p) => {
            driver::sqlite::query(p, &sql, &[JsonParam::Text(trigger_id.into())], 30_000).await?
        }
    };
    Ok(result
        .rows
        .first()
        .and_then(|r| r.0.first().map(|v| v.to_json()))
        .and_then(|v| v.as_str().map(|s| s.to_string())))
}

pub async fn write_cursor(
    pool: &Pool,
    table: &str,
    trigger_id: &str,
    cursor: &str,
) -> Result<(), DbError> {
    // `col` rather than `cursor` here: the function parameter is also named
    // `cursor` (the cursor value), so we must avoid shadowing it — otherwise
    // the bind below sends the column-name string instead of the value.
    let col = cursor_col(pool);
    let sql = match pool {
        Pool::Postgres(_) => format!(
            "INSERT INTO {table} (trigger_id, {col}, updated_at) VALUES ($1, $2, NOW()) \
             ON CONFLICT (trigger_id) DO UPDATE SET {col} = EXCLUDED.{col}, updated_at = NOW()"
        ),
        Pool::Mysql(_) => format!(
            "INSERT INTO {table} (trigger_id, {col}, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP(6)) \
             ON DUPLICATE KEY UPDATE {col} = VALUES({col}), updated_at = CURRENT_TIMESTAMP(6)"
        ),
        Pool::Sqlite(_) => format!(
            "INSERT INTO {table} (trigger_id, {col}, updated_at) VALUES (?, ?, datetime('now')) \
             ON CONFLICT(trigger_id) DO UPDATE SET {col} = excluded.{col}, updated_at = datetime('now')"
        ),
    };
    let params = vec![
        JsonParam::Text(trigger_id.into()),
        JsonParam::Text(cursor.into()),
    ];
    match pool {
        Pool::Postgres(p) => driver::postgres::execute(p, &sql, &params, &[])
            .await
            .map(|_| ()),
        Pool::Mysql(p) => driver::mysql::execute(p, &sql, &params, &[])
            .await
            .map(|_| ()),
        Pool::Sqlite(p) => driver::sqlite::execute(p, &sql, &params, &[])
            .await
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::pool::SqlitePool;

    fn pool() -> SqlitePool {
        SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_table_creates_in_sqlite() {
        let p = Pool::Sqlite(pool());
        ensure_table(&p, DEFAULT_CURSOR_TABLE).await.unwrap();
        // running again is idempotent
        ensure_table(&p, DEFAULT_CURSOR_TABLE).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_returns_none_for_unknown_trigger() {
        let p = Pool::Sqlite(pool());
        ensure_table(&p, DEFAULT_CURSOR_TABLE).await.unwrap();
        let v = read_cursor(&p, DEFAULT_CURSOR_TABLE, "trig-1")
            .await
            .unwrap();
        assert!(v.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_table_survives_external_drop() {
        // Regression: a process-wide ensure_table cache previously made the
        // second call a no-op, but if the table was dropped externally
        // between calls (test harness SCHEMA_RESET, operator action, ...)
        // every subsequent poll silently failed. Removing the cache makes
        // ensure_table idempotent under external drops.
        let p = Pool::Sqlite(pool());
        let table = "test_drop_resilience_xj";
        ensure_table(&p, table).await.unwrap();
        // External drop simulation.
        match &p {
            Pool::Sqlite(sp) => {
                crate::driver::sqlite::execute(
                    sp,
                    &format!("DROP TABLE IF EXISTS {table}"),
                    &[],
                    &[],
                )
                .await
                .unwrap();
            }
            _ => unreachable!(),
        }
        // Second call must re-create — not silently believe it's already done.
        ensure_table(&p, table).await.unwrap();
        // Verify the table exists by writing+reading a cursor row.
        write_cursor(&p, table, "trig-survive", "1").await.unwrap();
        let v = read_cursor(&p, table, "trig-survive").await.unwrap();
        assert_eq!(v.as_deref(), Some("1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn write_then_read_round_trips() {
        let p = Pool::Sqlite(pool());
        ensure_table(&p, DEFAULT_CURSOR_TABLE).await.unwrap();
        write_cursor(&p, DEFAULT_CURSOR_TABLE, "trig-1", "42")
            .await
            .unwrap();
        let v = read_cursor(&p, DEFAULT_CURSOR_TABLE, "trig-1")
            .await
            .unwrap();
        assert_eq!(v.as_deref(), Some("42"));
        // overwrite
        write_cursor(&p, DEFAULT_CURSOR_TABLE, "trig-1", "100")
            .await
            .unwrap();
        let v = read_cursor(&p, DEFAULT_CURSOR_TABLE, "trig-1")
            .await
            .unwrap();
        assert_eq!(v.as_deref(), Some("100"));
    }
}
