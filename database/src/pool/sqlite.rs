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
            Err(e) => Err(classify_acquire_error(&e.to_string(), db_name, timeout)),
        }
    }
}

/// `r2d2::get_timeout` returns one error type (`r2d2::Error`) for both
/// "no connection became free in time" and "the underlying connection
/// manager kept failing to open a connection until we hit the timeout".
/// Collapsing both to `PoolTimeout` masks misconfiguration (bad SQLite
/// path, missing parent dir, locked db) as pool exhaustion. r2d2's
/// `Display` writes `"timed out waiting for connection"` for the pure
/// timeout case and `"timed out waiting for connection: <inner>"` when
/// the most recent connection attempt left a failure on the pool's
/// internal `last_error` slot — the `: ` separator is the discriminator.
/// `r2d2::Error::source()` is the default `None` so the reviewer's
/// suggested `source().is_none()` check is a no-op against this crate
/// version (verified against r2d2-0.8.10/src/lib.rs:567-571).
///
/// Takes the formatted message rather than the `r2d2::Error` directly so
/// the classification logic can be unit-tested without constructing real
/// r2d2 errors (the inner field is private).
fn classify_acquire_error(display_msg: &str, db: String, timeout: Duration) -> DbError {
    if let Some((_, inner)) = display_msg.split_once(": ") {
        DbError::DriverError {
            driver: "sqlite".into(),
            code: None,
            message: format!("pool acquire failed: {inner}"),
            failed_index: None,
        }
    } else {
        DbError::PoolTimeout {
            db,
            waited_ms: timeout.as_millis() as u64,
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

    /// Regression: previously `Err(_) => DbError::PoolTimeout { .. }`
    /// collapsed every r2d2 acquire failure into a "pool saturated" error,
    /// even when the actual cause was the connection manager failing to
    /// open the database (e.g., parent directory missing, permissions,
    /// locked file). Operators staring at PoolTimeout would scale the pool
    /// up forever while the real fix was a path/perms issue. The classifier
    /// now inspects r2d2's Display string to distinguish the two cases.
    /// Tested at the helper boundary because r2d2_sqlite opens connections
    /// at pool-init (build) time — bad paths fail in `SqlitePool::new`
    /// before reaching `acquire()`, so we can't drive the live path with
    /// a dummy file. The helper is what carries the bug-fix logic.
    #[test]
    fn classify_acquire_error_with_inner_reason_returns_driver_error() {
        let err = classify_acquire_error(
            "timed out waiting for connection: unable to open database file",
            "primary".into(),
            Duration::from_millis(100),
        );
        match err {
            DbError::DriverError {
                driver, message, ..
            } => {
                assert_eq!(driver, "sqlite");
                assert!(
                    message.contains("unable to open database file"),
                    "got: {message}"
                );
            }
            other => panic!("expected DriverError, got {other:?}"),
        }
    }

    #[test]
    fn classify_acquire_error_pure_timeout_returns_pool_timeout() {
        let err = classify_acquire_error(
            "timed out waiting for connection",
            "primary".into(),
            Duration::from_millis(150),
        );
        match err {
            DbError::PoolTimeout { db, waited_ms } => {
                assert_eq!(db, "primary");
                assert_eq!(waited_ms, 150);
            }
            other => panic!("expected PoolTimeout, got {other:?}"),
        }
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
