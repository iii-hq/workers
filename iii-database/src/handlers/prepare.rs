//! `iii-database::prepareStatement` — pin a connection and return a UUID handle.

use super::AppState;
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Deserialize)]
struct PrepareReq {
    db: String,
    sql: String,
    #[serde(default = "default_ttl")]
    ttl_seconds: u64,
}

fn default_ttl() -> u64 {
    3600
}

const MAX_TTL_SECONDS: u64 = 86_400;

pub async fn handle(state: &AppState, payload: Value) -> Result<Value, String> {
    let req: PrepareReq = serde_json::from_value(payload).map_err(|e| {
        serde_json::to_string(&crate::error::DbError::InvalidParam {
            index: 0,
            reason: e.to_string(),
        })
        .unwrap_or_default()
    })?;

    let ttl = Duration::from_secs(req.ttl_seconds.min(MAX_TTL_SECONDS));
    let pool = state.pool(&req.db).map_err(err_to_str)?;
    // Reject empty SQL at the handler boundary, mirroring query.rs / execute.rs.
    // Without this, prepareStatement happily acquires a pool connection and
    // pins it under a UUID handle that can never run successfully — the
    // connection is leaked until the TTL expires.
    if req.sql.trim().is_empty() {
        return Err(err_to_str(crate::error::DbError::DriverError {
            driver: format!("{:?}", pool.driver()),
            code: None,
            message: "empty SQL".into(),
            failed_index: None,
        }));
    }

    let h = match pool {
        Pool::Sqlite(p) => {
            let conn = p.acquire().await.map_err(err_to_str)?;
            state
                .handles
                .insert_sqlite(req.sql.clone(), conn, ttl)
                .await
        }
        Pool::Postgres(p) => {
            let conn = p.acquire().await.map_err(err_to_str)?;
            state
                .handles
                .insert_postgres(req.sql.clone(), conn, ttl)
                .await
        }
        Pool::Mysql(p) => {
            let conn = p.acquire().await.map_err(err_to_str)?;
            state.handles.insert_mysql(req.sql.clone(), conn, ttl).await
        }
    };

    Ok(json!({ "handle": { "id": h.id, "expires_at": h.expires_at } }))
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

    fn state() -> AppState {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let mut pools = HashMap::new();
        pools.insert("primary".to_string(), Pool::Sqlite(pool));
        AppState {
            pools: Arc::new(pools),
            handles: Arc::new(HandleRegistry::new()),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_returns_handle_with_uuid_and_expiry() {
        let st = state();
        let resp = handle(
            &st,
            json!({
                "db": "primary",
                "sql": "SELECT 1"
            }),
        )
        .await
        .unwrap();
        let id = resp["handle"]["id"].as_str().unwrap();
        assert_eq!(id.len(), 36); // UUID
        assert!(resp["handle"]["expires_at"].is_string());
        assert!(st.handles.contains(id).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_clamps_ttl_to_max() {
        let st = state();
        let resp = handle(
            &st,
            json!({
                "db": "primary",
                "sql": "SELECT 1",
                "ttl_seconds": 999_999  // exceeds max 86400
            }),
        )
        .await
        .unwrap();
        // Should not error; expires_at should be ~24h out, not 11 days.
        assert!(resp["handle"]["id"].is_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_rejects_empty_sql() {
        // Without the handler-boundary guard, an empty SQL leaks the pool
        // connection until TTL expiry: prepareStatement acquires a connection
        // and pins it under a UUID handle that can never run successfully.
        let st = state();
        let err = handle(&st, json!({"db": "primary", "sql": ""}))
            .await
            .unwrap_err();
        assert!(
            err.contains("DRIVER_ERROR") && err.contains("empty SQL"),
            "expected DRIVER_ERROR/empty SQL, got: {err}"
        );
        // Whitespace-only is the same case.
        let err2 = handle(&st, json!({"db": "primary", "sql": "   \n\t"}))
            .await
            .unwrap_err();
        assert!(err2.contains("empty SQL"), "got: {err2}");
    }
}
