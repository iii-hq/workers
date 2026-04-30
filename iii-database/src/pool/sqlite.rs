//! SQLite pool wrapping `r2d2_sqlite`. Calls cross `spawn_blocking`.

use crate::config::PoolConfig;
use crate::error::DbError;
use r2d2::{Pool as R2Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct SqlitePool {
    inner: Arc<R2Pool<SqliteConnectionManager>>,
    db_name: Arc<str>,
    acquire_timeout: Duration,
}

/// A held connection from the pool. Closures run synchronously; callers wrap
/// in `tokio::task::spawn_blocking`.
#[derive(Debug)]
pub struct SqliteConn {
    conn: PooledConnection<SqliteConnectionManager>,
}

impl SqliteConn {
    pub fn with<R>(&self, f: impl FnOnce(&rusqlite::Connection) -> R) -> R {
        f(&self.conn)
    }

    pub fn with_mut<R>(&mut self, f: impl FnOnce(&mut rusqlite::Connection) -> R) -> R {
        f(&mut self.conn)
    }
}

impl SqlitePool {
    pub fn new(url: &str, pool_cfg: &PoolConfig) -> Result<Self, DbError> {
        let path = url.strip_prefix("sqlite:").unwrap_or(url);
        let manager = if path == ":memory:" || path.starts_with(":memory:") {
            SqliteConnectionManager::memory()
        } else {
            SqliteConnectionManager::file(path)
        };
        let inner = R2Pool::builder()
            .max_size(pool_cfg.max)
            .idle_timeout(Some(Duration::from_millis(pool_cfg.idle_timeout_ms)))
            .build(manager)
            .map_err(|e| DbError::ConfigError {
                message: format!("sqlite pool init: {e}"),
            })?;
        Ok(Self {
            inner: Arc::new(inner),
            db_name: Arc::from("(unset)"),
            acquire_timeout: Duration::from_millis(pool_cfg.acquire_timeout_ms),
        })
    }

    /// Tag the pool with a config name for error messages. Called by `pool::build`.
    pub fn with_db_name(mut self, name: &str) -> Self {
        self.db_name = Arc::from(name);
        self
    }

    pub async fn acquire(&self) -> Result<SqliteConn, DbError> {
        let pool = Arc::clone(&self.inner);
        let timeout = self.acquire_timeout;
        let db_name = self.db_name.to_string();
        let res = tokio::task::spawn_blocking(move || pool.get_timeout(timeout))
            .await
            .map_err(|e| DbError::DriverError {
                driver: "sqlite".into(),
                code: None,
                message: format!("spawn_blocking join: {e}"),
                failed_index: None,
            })?;
        match res {
            Ok(conn) => Ok(SqliteConn { conn }),
            Err(_) => Err(DbError::PoolTimeout {
                db: db_name,
                waited_ms: timeout.as_millis() as u64,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;

    #[tokio::test(flavor = "multi_thread")]
    async fn in_memory_pool_acquires_a_connection() {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let conn = pool.acquire().await.unwrap();
        let result: i64 = tokio::task::spawn_blocking(move || {
            conn.with(|c| c.query_row("SELECT 1", [], |row| row.get(0)))
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(result, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pool_timeout_when_max_one_and_held() {
        let pool = SqlitePool::new(
            "sqlite::memory:",
            &PoolConfig {
                max: 1,
                idle_timeout_ms: 30_000,
                acquire_timeout_ms: 50,
            },
        )
        .unwrap();
        let _held = pool.acquire().await.unwrap();
        let err = pool.acquire().await.unwrap_err();
        match err {
            crate::error::DbError::PoolTimeout { waited_ms, .. } => assert!(waited_ms >= 50),
            other => panic!("expected PoolTimeout, got {other:?}"),
        }
    }
}
