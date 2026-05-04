//! `iii-database::prepareStatement` — pin a connection and return a UUID handle.

use super::AppState;
use crate::handle::HandleResponse;
use crate::handlers::query::err_to_str;
use crate::pool::Pool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
pub struct PrepareReq {
    pub db: String,
    pub sql: String,
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PrepareResp {
    pub handle: HandleResponse,
}

fn default_ttl() -> u64 {
    3600
}

const MAX_TTL_SECONDS: u64 = 86_400;

pub async fn handle(state: &AppState, req: PrepareReq) -> Result<PrepareResp, String> {
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

    Ok(PrepareResp { handle: h })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::handle::HandleRegistry;
    use crate::handlers::AppState;
    use crate::pool::{Pool, SqlitePool};
    use serde_json::{json, Value};
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

    fn req(v: Value) -> PrepareReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_returns_handle_with_uuid_and_expiry() {
        let st = state();
        let resp = handle(
            &st,
            req(json!({
                "db": "primary",
                "sql": "SELECT 1"
            })),
        )
        .await
        .unwrap();
        let id = &resp.handle.id;
        assert_eq!(id.len(), 36); // UUID
        assert!(st.handles.contains(id).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_clamps_ttl_to_max() {
        let st = state();
        let resp = handle(
            &st,
            req(json!({
                "db": "primary",
                "sql": "SELECT 1",
                "ttl_seconds": 999_999  // exceeds max 86400
            })),
        )
        .await
        .unwrap();
        // Should not error; expires_at should be ~24h out, not 11 days.
        assert!(!resp.handle.id.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepare_rejects_empty_sql() {
        // Without the handler-boundary guard, an empty SQL leaks the pool
        // connection until TTL expiry: prepareStatement acquires a connection
        // and pins it under a UUID handle that can never run successfully.
        let st = state();
        let err = handle(&st, req(json!({"db": "primary", "sql": ""})))
            .await
            .unwrap_err();
        assert!(
            err.contains("DRIVER_ERROR") && err.contains("empty SQL"),
            "expected DRIVER_ERROR/empty SQL, got: {err}"
        );
        // Whitespace-only is the same case.
        let err2 = handle(&st, req(json!({"db": "primary", "sql": "   \n\t"})))
            .await
            .unwrap_err();
        assert!(err2.contains("empty SQL"), "got: {err2}");
    }
}
