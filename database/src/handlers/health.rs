//! `database::health` — live operational state, honestly reported.
//!
//! The design point is `ProbeResult`. Each section answers separately, so a
//! caller can tell "sqlite has no equivalent of `pg_stat_activity`" from
//! "there are zero active queries" from "this role may not read
//! `pg_stat_activity`". Collapsing those into an empty list would render a
//! confidently wrong panel, and a restricted application role is the common
//! case rather than the exception — so one probe being denied never fails the
//! whole call.
//!
//! Boundary against `database::testConnection`: that probes a *candidate* URL
//! that is not configured yet. This reports the live state of a pool that
//! already exists, and deliberately accepts no URL.

use super::query::{self, err_to_str, QueryReq};
use super::AppState;
use crate::config::DriverKind;
use crate::error::DbError;
use crate::pool::PoolStats;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_timeout() -> u64 {
    15_000
}

/// One section of the report.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ProbeResult<T> {
    /// The driver answered.
    Available { data: T },
    /// The driver has no equivalent of this concept.
    Unsupported { reason: String },
    /// The driver has it, but this role may not read it.
    Denied { reason: String },
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ActiveQuery {
    pub id: String,
    pub sql: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TableSize {
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_estimate: Option<i64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LockInfo {
    pub blocked_id: String,
    pub blocked_sql: String,
    pub blocking_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocking_sql: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CacheStats {
    /// Fraction of block reads served from cache. A healthy OLTP database
    /// usually sits well above 0.99.
    pub hit_ratio: f64,
    pub blocks_hit: i64,
    pub blocks_read: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HealthReq {
    #[serde(default)]
    pub db: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResp {
    pub db: String,
    pub driver: String,
    pub worker_version: String,
    pub pool: PoolStats,
    pub active_queries: ProbeResult<Vec<ActiveQuery>>,
    pub table_sizes: ProbeResult<Vec<TableSize>>,
    pub locks: ProbeResult<Vec<LockInfo>>,
    pub cache: ProbeResult<CacheStats>,
}

fn unsupported<T>(driver: DriverKind, what: &str) -> ProbeResult<T> {
    ProbeResult::Unsupported {
        reason: format!("{driver:?} has no equivalent of {what}").to_lowercase(),
    }
}

/// Turn a probe failure into a per-section result rather than failing the
/// whole call. A permission error on one view must not hide the others.
fn probe<T>(outcome: Result<T, String>) -> ProbeResult<T> {
    match outcome {
        Ok(data) => ProbeResult::Available { data },
        Err(reason) => ProbeResult::Denied { reason },
    }
}

async fn rows(
    state: &AppState,
    db: &str,
    sql: &str,
    timeout_ms: u64,
) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    Ok(query::handle(
        state,
        QueryReq {
            db: Some(db.to_string()),
            sql: sql.to_string(),
            params: vec![],
            timeout_ms,
            record_history: false,
        },
    )
    .await?
    .rows)
}

fn s_at(r: &serde_json::Map<String, Value>, k: &str) -> Option<String> {
    match r.get(k) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn i_at(r: &serde_json::Map<String, Value>, k: &str) -> Option<i64> {
    match r.get(k) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

pub async fn handle(state: &AppState, req: HealthReq) -> Result<HealthResp, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let pool = state.pool(&db).await.map_err(err_to_str)?;
    let driver = pool.driver();
    let t = req.timeout_ms;

    let (active_queries, table_sizes, locks, cache) = match driver {
        DriverKind::Postgres => (
            probe(pg_active(state, &db, t).await),
            probe(pg_sizes(state, &db, t).await),
            probe(pg_locks(state, &db, t).await),
            probe(pg_cache(state, &db, t).await),
        ),
        DriverKind::Mysql => (
            probe(mysql_active(state, &db, t).await),
            probe(mysql_sizes(state, &db, t).await),
            unsupported(driver, "a queryable lock-wait graph"),
            unsupported(driver, "a per-database buffer-pool hit ratio"),
        ),
        DriverKind::Sqlite => (
            // SQLite runs in-process: there is no server holding sessions.
            unsupported(driver, "server-side sessions"),
            unsupported(driver, "per-table size accounting"),
            unsupported(driver, "a queryable lock table"),
            unsupported(driver, "a shared buffer cache"),
        ),
    };

    Ok(HealthResp {
        db,
        driver: format!("{driver:?}").to_lowercase(),
        worker_version: env!("CARGO_PKG_VERSION").to_string(),
        pool: pool.stats(),
        active_queries,
        table_sizes,
        locks,
        cache,
    })
}

/* ---------------- postgres ---------------- */

async fn pg_active(state: &AppState, db: &str, t: u64) -> Result<Vec<ActiveQuery>, String> {
    let r = rows(
        state,
        db,
        "SELECT pid::text AS id, query AS sql, state, usename AS usr, \
                (EXTRACT(EPOCH FROM (now() - query_start)) * 1000)::bigint AS duration_ms \
         FROM pg_stat_activity \
         WHERE datname = current_database() AND pid <> pg_backend_pid() \
           AND state <> 'idle' \
         ORDER BY query_start",
        t,
    )
    .await?;
    Ok(r.iter()
        .filter_map(|x| {
            Some(ActiveQuery {
                id: s_at(x, "id")?,
                sql: s_at(x, "sql").unwrap_or_default(),
                state: s_at(x, "state"),
                duration_ms: i_at(x, "duration_ms"),
                user: s_at(x, "usr"),
            })
        })
        .collect())
}

async fn pg_sizes(state: &AppState, db: &str, t: u64) -> Result<Vec<TableSize>, String> {
    let r = rows(
        state,
        db,
        "SELECT n.nspname AS schema_name, c.relname AS table_name, \
                pg_total_relation_size(c.oid)::bigint AS total_bytes, \
                pg_indexes_size(c.oid)::bigint AS index_bytes, \
                c.reltuples::bigint AS row_estimate \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE c.relkind IN ('r', 'p') \
           AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
         ORDER BY pg_total_relation_size(c.oid) DESC LIMIT 100",
        t,
    )
    .await?;
    Ok(r.iter()
        .filter_map(|x| {
            Some(TableSize {
                table: s_at(x, "table_name")?,
                schema: s_at(x, "schema_name"),
                total_bytes: i_at(x, "total_bytes"),
                index_bytes: i_at(x, "index_bytes"),
                row_estimate: i_at(x, "row_estimate").filter(|v| *v >= 0),
            })
        })
        .collect())
}

/// Resolves blockers by joining `pg_locks` to itself. Deliberately avoids
/// `pg_blocking_pids()`, which returns `int[]` — and `RowValue` has no array
/// variant, so that column would fail to decode.
async fn pg_locks(state: &AppState, db: &str, t: u64) -> Result<Vec<LockInfo>, String> {
    let r = rows(
        state,
        db,
        "SELECT w.pid::text AS blocked_id, wa.query AS blocked_sql, \
                b.pid::text AS blocking_id, ba.query AS blocking_sql, \
                COALESCE(c.relname, '') AS relation \
         FROM pg_locks w \
         JOIN pg_locks b ON b.granted AND NOT w.granted \
              AND b.pid <> w.pid \
              AND b.locktype = w.locktype \
              AND b.database IS NOT DISTINCT FROM w.database \
              AND b.relation IS NOT DISTINCT FROM w.relation \
              AND b.transactionid IS NOT DISTINCT FROM w.transactionid \
         JOIN pg_stat_activity wa ON wa.pid = w.pid \
         LEFT JOIN pg_stat_activity ba ON ba.pid = b.pid \
         LEFT JOIN pg_class c ON c.oid = w.relation \
         WHERE NOT w.granted",
        t,
    )
    .await?;
    Ok(r.iter()
        .filter_map(|x| {
            Some(LockInfo {
                blocked_id: s_at(x, "blocked_id")?,
                blocked_sql: s_at(x, "blocked_sql").unwrap_or_default(),
                blocking_id: s_at(x, "blocking_id")?,
                blocking_sql: s_at(x, "blocking_sql"),
                relation: s_at(x, "relation").filter(|s| !s.is_empty()),
            })
        })
        .collect())
}

async fn pg_cache(state: &AppState, db: &str, t: u64) -> Result<CacheStats, String> {
    let r = rows(
        state,
        db,
        "SELECT COALESCE(SUM(heap_blks_hit), 0)::bigint AS hit, \
                COALESCE(SUM(heap_blks_read), 0)::bigint AS rd \
         FROM pg_statio_user_tables",
        t,
    )
    .await?;
    let row = r.first().cloned().unwrap_or_default();
    let hit = i_at(&row, "hit").unwrap_or(0);
    let read = i_at(&row, "rd").unwrap_or(0);
    let total = hit + read;
    Ok(CacheStats {
        // No reads yet is not a 0% hit rate; report it as perfect rather than
        // as an alarming zero.
        hit_ratio: if total == 0 {
            1.0
        } else {
            hit as f64 / total as f64
        },
        blocks_hit: hit,
        blocks_read: read,
    })
}

/* ---------------- mysql ---------------- */

async fn mysql_active(state: &AppState, db: &str, t: u64) -> Result<Vec<ActiveQuery>, String> {
    let r = rows(
        state,
        db,
        "SELECT ID AS id, INFO AS sql_text, STATE AS state, USER AS usr, \
                TIME * 1000 AS duration_ms \
         FROM information_schema.PROCESSLIST \
         WHERE DB = DATABASE() AND COMMAND <> 'Sleep' AND ID <> CONNECTION_ID() \
         ORDER BY TIME DESC",
        t,
    )
    .await?;
    Ok(r.iter()
        .filter_map(|x| {
            Some(ActiveQuery {
                id: s_at(x, "id")?,
                sql: s_at(x, "sql_text").unwrap_or_default(),
                state: s_at(x, "state"),
                duration_ms: i_at(x, "duration_ms"),
                user: s_at(x, "usr"),
            })
        })
        .collect())
}

async fn mysql_sizes(state: &AppState, db: &str, t: u64) -> Result<Vec<TableSize>, String> {
    let r = rows(
        state,
        db,
        "SELECT TABLE_NAME AS table_name, \
                (DATA_LENGTH + INDEX_LENGTH) AS total_bytes, \
                INDEX_LENGTH AS index_bytes, TABLE_ROWS AS row_estimate \
         FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' \
         ORDER BY (DATA_LENGTH + INDEX_LENGTH) DESC LIMIT 100",
        t,
    )
    .await?;
    Ok(r.iter()
        .filter_map(|x| {
            Some(TableSize {
                table: s_at(x, "table_name")?,
                schema: None,
                total_bytes: i_at(x, "total_bytes"),
                index_bytes: i_at(x, "index_bytes"),
                row_estimate: i_at(x, "row_estimate"),
            })
        })
        .collect())
}

/* ---------------- terminateQuery ---------------- */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TerminateReq {
    #[serde(default)]
    pub db: Option<String>,
    /// Backend pid (postgres) or connection id (mysql), as reported by
    /// `database::health`.
    pub id: String,
    /// Ask the backend to cancel the running statement but keep the session.
    /// The default terminates the session outright.
    #[serde(default)]
    pub cancel_only: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TerminateResp {
    pub id: String,
    pub terminated: bool,
}

/// A separate function from `health` on purpose: this is a write, and a
/// read-only viewer must not be able to synthesise it from a report.
pub async fn terminate(state: &AppState, req: TerminateReq) -> Result<TerminateResp, String> {
    let db = state.resolve_db(req.db).await.map_err(err_to_str)?;
    let pool = state.pool(&db).await.map_err(err_to_str)?;
    let driver = pool.driver();

    // The id is interpolated, so it must be exactly a number — never trust it
    // as an identifier.
    let id: i64 = req.id.trim().parse().map_err(|_| {
        err_to_str(DbError::InvalidParam {
            index: 0,
            reason: format!("`{}` is not a backend id", req.id),
        })
    })?;

    let sql = match (driver, req.cancel_only) {
        (DriverKind::Postgres, true) => format!("SELECT pg_cancel_backend({id}) AS ok"),
        (DriverKind::Postgres, false) => format!("SELECT pg_terminate_backend({id}) AS ok"),
        (DriverKind::Mysql, true) => format!("KILL QUERY {id}"),
        (DriverKind::Mysql, false) => format!("KILL CONNECTION {id}"),
        (DriverKind::Sqlite, _) => {
            return Err(err_to_str(DbError::InvalidParam {
                index: 0,
                reason: "sqlite runs in-process and has no sessions to terminate".into(),
            }))
        }
    };

    let terminated = match driver {
        DriverKind::Postgres => {
            let r = rows(state, &db, &sql, req.timeout_ms).await?;
            r.first()
                .and_then(|x| x.get("ok"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        }
        // KILL returns no result set; reaching here without an error is the
        // signal.
        _ => {
            super::execute::handle(
                state,
                super::execute::ExecuteReq {
                    db: Some(db.clone()),
                    sql: sql.clone(),
                    params: vec![],
                    returning: vec![],
                },
            )
            .await?;
            true
        }
    };

    Ok(TerminateResp {
        id: req.id,
        terminated,
    })
}
