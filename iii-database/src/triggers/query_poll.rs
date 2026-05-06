//! query-poll trigger — cursor-based polling loop.
//!
//! On each tick:
//!   1. Read the cursor from `__iii_cursors` for this `trigger_id`.
//!   2. Run the user SQL with the cursor bound as the single positional parameter.
//!   3. If rows returned: dispatch a batch to the engine.
//!   4. On `ack: true`, write the max of `cursor_column` back to `__iii_cursors`.

use crate::cursor;
use crate::driver;
use crate::error::DbError;
use crate::pool::Pool;
use crate::value::{JsonParam, RowValue};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct QueryPollConfig {
    pub trigger_id: String,
    #[serde(rename = "db")]
    pub db_name: String,
    pub sql: String,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    pub cursor_column: String,
    #[serde(default = "default_cursor_table")]
    pub cursor_table: String,
}

fn default_interval_ms() -> u64 {
    1000
}
fn default_cursor_table() -> String {
    cursor::DEFAULT_CURSOR_TABLE.to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatchedBatch {
    pub db: String,
    pub rows: Vec<serde_json::Map<String, Value>>,
    pub cursor: Option<String>,
    pub polled_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DispatchAck {
    #[serde(default = "default_ack")]
    pub ack: bool,
    #[serde(default)]
    pub commit_cursor: Option<String>,
}

fn default_ack() -> bool {
    true
}

#[async_trait]
pub trait Dispatch: Send + Sync {
    async fn dispatch(&self, batch: DispatchedBatch) -> Result<DispatchAck, DbError>;
}

impl QueryPollConfig {
    /// Validate operator-supplied identifiers that get interpolated into
    /// SQL strings (currently `cursor_table`). Call this once at config-load
    /// time; `run_one_tick` also defends in depth on the chance the config
    /// was constructed in code without going through `from_yaml`.
    pub fn validate(&self) -> Result<(), DbError> {
        crate::config::validate_sql_identifier(&self.cursor_table).map_err(|e| {
            DbError::ConfigError {
                message: format!("query-poll cursor_table: {e}"),
            }
        })?;
        Ok(())
    }
}

/// Compute the max cursor value across all rows in a poll batch.
///
/// Two-pass design keeps the integer hot path zero-alloc:
///   1. Try to compute `i64::max` over every row's cursor cell. Pure
///      `RowValue::Int`/`BigInt` go straight in; `Text`/`Decimal` are
///      `parse::<i64>()`'d (handles stringly-typed integer cursors).
///   2. If any row's cell can't be coerced to i64, fall back to lexicographic
///      max — but only stringify cells that aren't already textual.
///
/// On the common integer-cursor case (the example SQL in the README), this
/// allocates exactly one String at the end (the i64::to_string of the max),
/// regardless of batch size.
fn compute_cursor_max(rows: &[crate::driver::Row], col_idx: usize) -> Option<String> {
    let mut int_max: Option<i64> = None;
    let mut all_ints = true;
    for row in rows {
        let Some(v) = row.0.get(col_idx) else {
            continue;
        };
        let parsed: Option<i64> = match v {
            RowValue::Int(n) | RowValue::BigInt(n) => Some(*n),
            RowValue::Text(s) | RowValue::Decimal(s) => s.parse::<i64>().ok(),
            _ => None,
        };
        match parsed {
            Some(n) => int_max = Some(int_max.map_or(n, |m| m.max(n))),
            None => {
                all_ints = false;
                break;
            }
        }
    }
    if all_ints {
        return int_max.map(|n| n.to_string());
    }
    // Fallback: stringly-typed cursor (UUIDs, ISO-8601, etc.). Borrow `&str`
    // from `Text`/`Decimal` cells; for variants without a native string repr,
    // stringify just-in-time. Cow keeps the per-row branch alloc-free on the
    // common path where every row is text.
    use std::borrow::Cow;
    let mut best: Option<Cow<'_, str>> = None;
    for row in rows {
        let Some(v) = row.0.get(col_idx) else {
            continue;
        };
        let cur: Cow<'_, str> = match v {
            RowValue::Text(s) | RowValue::Decimal(s) => Cow::Borrowed(s.as_str()),
            other => Cow::Owned(other.to_json().to_string().trim_matches('"').to_string()),
        };
        best = Some(match best {
            Some(prev) if prev.as_ref() >= cur.as_ref() => prev,
            _ => cur,
        });
    }
    best.map(Cow::into_owned)
}

pub async fn run_one_tick(
    pool: &Pool,
    cfg: &QueryPollConfig,
    dispatch: Arc<dyn Dispatch>,
) -> Result<(), DbError> {
    cfg.validate()?;
    cursor::ensure_table(pool, &cfg.cursor_table).await?;
    let cur = cursor::read_cursor(pool, &cfg.cursor_table, &cfg.trigger_id).await?;
    let cur_param = match cur.as_deref() {
        Some(s) => match s.parse::<i64>() {
            Ok(n) => JsonParam::Int(n),
            Err(_) => JsonParam::Text(s.to_string()),
        },
        None => JsonParam::Null,
    };

    let result = match pool {
        Pool::Sqlite(p) => driver::sqlite::query(p, &cfg.sql, &[cur_param], 30_000).await?,
        Pool::Postgres(p) => driver::postgres::query(p, &cfg.sql, &[cur_param], 30_000).await?,
        Pool::Mysql(p) => driver::mysql::query(p, &cfg.sql, &[cur_param], 30_000).await?,
    };

    if result.rows.is_empty() {
        return Ok(());
    }

    let col_idx = result
        .columns
        .iter()
        .position(|c| c.name == cfg.cursor_column)
        .ok_or_else(|| DbError::ConfigError {
            message: format!(
                "cursor_column `{}` not found in result columns",
                cfg.cursor_column
            ),
        })?;

    let max_cursor: Option<String> = compute_cursor_max(&result.rows, col_idx);

    let json_rows = crate::handlers::query_rows_to_objects(&result.columns, result.rows);

    let batch = DispatchedBatch {
        db: cfg.db_name.clone(),
        rows: json_rows,
        cursor: max_cursor.clone(),
        polled_at: chrono::Utc::now(),
    };

    let ack = dispatch.dispatch(batch).await?;
    if ack.ack {
        let new_cursor = ack.commit_cursor.or(max_cursor);
        if let Some(c) = new_cursor {
            cursor::write_cursor(pool, &cfg.cursor_table, &cfg.trigger_id, &c).await?;
        }
    }
    Ok(())
}

pub async fn run_loop(pool: Pool, cfg: QueryPollConfig, dispatch: Arc<dyn Dispatch>) {
    let mut interval = tokio::time::interval(Duration::from_millis(cfg.interval_ms));
    // Drop ticks that fire while a previous tick is still running, instead of
    // bursting to catch up. Without this, a slow query would queue up multiple
    // back-to-back polls and hammer the DB once the slow tick completes.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(e) = run_one_tick(&pool, &cfg, dispatch.clone()).await {
            tracing::warn!(trigger_id = %cfg.trigger_id, error = ?e, "query-poll tick failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::pool::{Pool, SqlitePool};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CapturingDispatch {
        calls: Mutex<Vec<DispatchedBatch>>,
        ack: bool,
    }

    #[async_trait]
    impl Dispatch for CapturingDispatch {
        async fn dispatch(
            &self,
            batch: DispatchedBatch,
        ) -> Result<DispatchAck, crate::error::DbError> {
            self.calls.lock().unwrap().push(batch);
            Ok(DispatchAck {
                ack: self.ack,
                commit_cursor: None,
            })
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_emits_new_rows_and_advances_cursor_on_ack() {
        let p = Pool::Sqlite(SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap());

        // Setup table
        let s = match &p {
            Pool::Sqlite(s) => s,
            _ => unreachable!(),
        };
        crate::driver::sqlite::execute(
            s,
            "CREATE TABLE outbox (id INTEGER PRIMARY KEY, body TEXT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        // Use separate inserts because driver::sqlite::execute uses Connection::execute (single statement)
        for body in &["a", "b", "c"] {
            crate::driver::sqlite::execute(
                s,
                "INSERT INTO outbox (body) VALUES (?)",
                &[JsonParam::Text((*body).into())],
                &[],
            )
            .await
            .unwrap();
        }

        let dispatch = Arc::new(CapturingDispatch {
            calls: Default::default(),
            ack: true,
        });
        let cfg = QueryPollConfig {
            trigger_id: "trig-1".into(),
            db_name: "primary".into(),
            sql: "SELECT id, body FROM outbox WHERE id > COALESCE(?, 0) ORDER BY id LIMIT 50"
                .into(),
            interval_ms: 25,
            cursor_column: "id".into(),
            cursor_table: crate::cursor::DEFAULT_CURSOR_TABLE.into(),
        };

        run_one_tick(&p, &cfg, dispatch.clone()).await.unwrap();
        let calls_len = dispatch.calls.lock().unwrap().len();
        assert_eq!(calls_len, 1);
        assert_eq!(dispatch.calls.lock().unwrap()[0].rows.len(), 3);

        // Cursor should now be "3".
        let v = crate::cursor::read_cursor(&p, &cfg.cursor_table, &cfg.trigger_id)
            .await
            .unwrap();
        assert_eq!(v.as_deref(), Some("3"));

        // Second tick produces no new rows.
        run_one_tick(&p, &cfg, dispatch.clone()).await.unwrap();
        assert_eq!(
            dispatch.calls.lock().unwrap().len(),
            1,
            "no new rows should produce no dispatch"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_does_not_advance_on_nack() {
        let p = Pool::Sqlite(SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap());
        let s = match &p {
            Pool::Sqlite(s) => s,
            _ => unreachable!(),
        };
        crate::driver::sqlite::execute(
            s,
            "CREATE TABLE outbox (id INTEGER PRIMARY KEY, body TEXT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        crate::driver::sqlite::execute(s, "INSERT INTO outbox (body) VALUES ('x')", &[], &[])
            .await
            .unwrap();

        let dispatch = Arc::new(CapturingDispatch {
            calls: Default::default(),
            ack: false,
        });
        let cfg = QueryPollConfig {
            trigger_id: "trig-x".into(),
            db_name: "primary".into(),
            sql: "SELECT id, body FROM outbox WHERE id > COALESCE(?, 0) ORDER BY id".into(),
            interval_ms: 25,
            cursor_column: "id".into(),
            cursor_table: crate::cursor::DEFAULT_CURSOR_TABLE.into(),
        };
        run_one_tick(&p, &cfg, dispatch.clone()).await.unwrap();
        let v = crate::cursor::read_cursor(&p, &cfg.cursor_table, &cfg.trigger_id)
            .await
            .unwrap();
        assert!(v.is_none(), "cursor should not advance on nack");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_cursor_advances_numerically_across_digit_boundary() {
        // Regression: lexicographic max("9","10") == "9", which would replay
        // row 10 forever. The driver must compare cursor values numerically
        // when every value parses as i64.
        let p = Pool::Sqlite(SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap());
        let s = match &p {
            Pool::Sqlite(s) => s,
            _ => unreachable!(),
        };
        crate::driver::sqlite::execute(
            s,
            "CREATE TABLE outbox (id INTEGER PRIMARY KEY, body TEXT)",
            &[],
            &[],
        )
        .await
        .unwrap();
        // 12 rows so the batch crosses the 9→10 boundary.
        for i in 1..=12 {
            crate::driver::sqlite::execute(
                s,
                "INSERT INTO outbox (body) VALUES (?)",
                &[JsonParam::Text(format!("body-{i}"))],
                &[],
            )
            .await
            .unwrap();
        }

        let dispatch = Arc::new(CapturingDispatch {
            calls: Default::default(),
            ack: true,
        });
        let cfg = QueryPollConfig {
            trigger_id: "trig-num".into(),
            db_name: "primary".into(),
            sql: "SELECT id, body FROM outbox WHERE id > COALESCE(?, 0) ORDER BY id LIMIT 50"
                .into(),
            interval_ms: 25,
            cursor_column: "id".into(),
            cursor_table: crate::cursor::DEFAULT_CURSOR_TABLE.into(),
        };
        run_one_tick(&p, &cfg, dispatch.clone()).await.unwrap();
        let v = crate::cursor::read_cursor(&p, &cfg.cursor_table, &cfg.trigger_id)
            .await
            .unwrap();
        assert_eq!(
            v.as_deref(),
            Some("12"),
            "cursor must be the numeric max (12), not the lexicographic max (9)"
        );
    }
}
