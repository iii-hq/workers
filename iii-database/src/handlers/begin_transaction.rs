//! `iii-database::beginTransaction` — open an interactive transaction and
//! return a handle. The handle pins one pool connection inside a server-
//! side `BEGIN ... COMMIT/ROLLBACK` until either the matching
//! `commitTransaction` / `rollbackTransaction` call lands, or the
//! configurable per-tx timeout fires (background watcher in
//! `crate::transaction::TxRegistry::spawn_timeout_watcher`).

use super::AppState;
use crate::driver::{self, Isolation};
use crate::error::DbError;
use crate::handle::PinnedConn;
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use crate::transaction::{driver_system, TxHandleResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
pub struct BeginTxReq {
    pub db: String,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BeginTxResp {
    pub transaction: TxHandleResponse,
}

fn default_timeout_ms() -> u64 {
    30_000
}

/// Upper bound on `timeout_ms`. 5 minutes is the longest a single
/// transaction should reasonably hold a pool connection before the
/// timeout-driven rollback fires. Operators who need longer txns should
/// bump this constant rather than work around it with retries.
const MAX_TIMEOUT_MS: u64 = 300_000;

fn parse_isolation(s: Option<&str>) -> Result<Option<Isolation>, DbError> {
    match s {
        None => Ok(None),
        Some("read_committed") => Ok(Some(Isolation::ReadCommitted)),
        Some("repeatable_read") => Ok(Some(Isolation::RepeatableRead)),
        Some("serializable") => Ok(Some(Isolation::Serializable)),
        Some(other) => Err(DbError::InvalidParam {
            index: 0,
            reason: format!("unknown isolation `{other}`"),
        }),
    }
}

pub async fn handle(state: &AppState, req: BeginTxReq) -> Result<BeginTxResp, String> {
    let pool = state.pool(&req.db).map_err(err_to_str)?;
    let driver = pool.driver();
    let isolation = parse_isolation(req.isolation.as_deref()).map_err(err_to_str)?;
    let timeout_ms = req
        .timeout_ms
        .unwrap_or_else(default_timeout_ms)
        .min(MAX_TIMEOUT_MS);
    let timeout = Duration::from_millis(timeout_ms);

    // Acquire a connection from the pool and issue BEGIN on it. If BEGIN
    // fails the pinned conn drops here and returns to the pool — no leaked
    // half-open transaction. We log the failure with the SDK logger so it
    // shows up in the per-call OTel span.
    let mut pinned = match pool {
        Pool::Sqlite(p) => PinnedConn::Sqlite(Some(p.acquire().await.map_err(err_to_str)?)),
        Pool::Postgres(p) => PinnedConn::Postgres(p.acquire().await.map_err(err_to_str)?),
        Pool::Mysql(p) => PinnedConn::Mysql(p.acquire().await.map_err(err_to_str)?),
    };

    let begin_result = match &mut pinned {
        PinnedConn::Sqlite(slot) => driver::sqlite::tx_begin(slot, isolation).await,
        PinnedConn::Postgres(client) => driver::postgres::tx_begin(client, isolation).await,
        PinnedConn::Mysql(conn) => driver::mysql::tx_begin(conn, isolation).await,
    };
    if let Err(e) = begin_result {
        state.log.warn(
            "db_tx_begin_failed",
            Some(json!({
                "db.system": driver_system(driver),
                "db.name": req.db,
                "isolation": req.isolation,
                "error": serde_json::to_value(&e).ok(),
            })),
        );
        return Err(err_to_str(e));
    }

    let handle = state
        .transactions
        .insert(req.db.clone(), driver, pinned, timeout)
        .await;

    state.log.info(
        "db_tx_started",
        Some(json!({
            "db.system": driver_system(driver),
            "db.name": req.db,
            "db.operation": "BEGIN",
            "db.transaction.id": handle.id,
            "isolation": req.isolation,
            "timeout_ms": timeout_ms,
        })),
    );

    Ok(BeginTxResp {
        transaction: handle,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::handlers::AppState;
    use crate::pool::{Pool, SqlitePool};
    use crate::transaction::TxRegistry;
    use iii_sdk::Logger;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    pub fn state() -> AppState {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(pools),
            handles: Arc::new(HandleRegistry::new()),
            transactions: TxRegistry::new(),
            log: Logger::new(),
        }
    }

    fn req(v: Value) -> BeginTxReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn begin_returns_handle_and_records_in_registry() {
        let st = state();
        let resp = handle(&st, req(json!({ "db": "primary" }))).await.unwrap();
        assert_eq!(resp.transaction.id.len(), 36);
        assert_eq!(st.transactions.len().await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn begin_unknown_db_returns_unknown_db_error() {
        let st = state();
        let err = handle(&st, req(json!({ "db": "missing" })))
            .await
            .unwrap_err();
        assert!(err.contains("UNKNOWN_DB"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn begin_unknown_isolation_returns_invalid_param() {
        let st = state();
        let err = handle(
            &st,
            req(json!({ "db": "primary", "isolation": "bogus_level" })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("bogus_level"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn begin_clamps_timeout_to_max() {
        // Per-tx timeout caps at MAX_TIMEOUT_MS (5 min) regardless of caller input;
        // operators tweak the constant if they need longer.
        let st = state();
        let resp = handle(
            &st,
            req(json!({ "db": "primary", "timeout_ms": 999_999_999_u64 })),
        )
        .await
        .unwrap();
        let now = chrono::Utc::now();
        // expires_at should be within ~5 min from now, NOT ~11 days.
        let diff = (resp.transaction.expires_at - now).num_milliseconds();
        assert!(
            (0..=MAX_TIMEOUT_MS as i64 + 5_000).contains(&diff),
            "expires_at not clamped to MAX_TIMEOUT_MS; diff={diff}ms"
        );
    }
}
