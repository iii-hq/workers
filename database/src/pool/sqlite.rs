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
    /// Live occupancy from r2d2's own counters.
    pub fn stats(&self) -> crate::pool::PoolStats {
        let st = self.inner.state();
        crate::pool::PoolStats {
            max: self.inner.max_size(),
            size: Some(st.connections),
            idle: Some(st.idle_connections),
            waiting: None,
        }
    }

    pub fn new(url: &str, pool_cfg: &PoolConfig) -> Result<Self, DbError> {
        let path = url.strip_prefix("sqlite:").unwrap_or(url);
        let manager = if path == ":memory:" || path.starts_with(":memory:") {
            SqliteConnectionManager::memory()
        } else {
            // SQLite opens (and, with the default flags, creates) the database
            // *file*, but it will NOT create missing parent directories — a
            // fresh `sqlite:./data/iii.db` boot where `./data` does not yet
            // exist fails at pool-build time with `unable to open database
            // file`. r2d2 eagerly opens connections in `build()`, so this
            // surfaces as a hard startup crash (exactly what broke the registry
            // publish CI: the worker is launched from a clean checkout with no
            // `data/` dir). Create the parent dir up front so the default
            // config — and any relative/nested sqlite path — boots cleanly.
            if let Some(parent) = parent_dir_to_create(path) {
                std::fs::create_dir_all(&parent).map_err(|e| DbError::ConfigError {
                    message: format!(
                        "sqlite: could not create parent directory {}: {e}",
                        parent.display()
                    ),
                })?;
            }
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
        let res = tokio::task::spawn_blocking(move || {
            let conn = pool.get_timeout(timeout).map_err(AcquireFailure::Pool)?;
            // Defense in depth against pool poisoning: if a previous holder
            // leaked an open transaction (the handler-boundary guard blocks
            // the known path, but drivers and future surfaces can regress),
            // roll it back instead of handing every later caller `cannot
            // start a transaction within a transaction` forever.
            if !conn.is_autocommit() {
                tracing::warn!(
                    "sqlite pool: connection acquired with an open transaction; rolling back"
                );
                let rollback = conn.execute_batch("ROLLBACK");
                // A failed or ineffective rollback means the connection is
                // still inside a transaction — handing it out would recreate
                // the exact poisoning this check exists to prevent. Refuse
                // this acquire; the connection returns to the pool and the
                // rollback is retried on its next checkout, so a transient
                // failure self-heals instead of poisoning forever.
                if rollback.is_err() || !conn.is_autocommit() {
                    return Err(AcquireFailure::StuckTransaction(
                        rollback.err().map(|e| e.to_string()),
                    ));
                }
            }
            Ok(conn)
        })
        .await
        .map_err(|e| DbError::DriverError {
            driver: "sqlite".into(),
            code: None,
            message: format!("spawn_blocking join: {e}"),
            failed_index: None,
        })?;
        match res {
            Ok(conn) => Ok(SqliteConn { conn }),
            Err(AcquireFailure::Pool(e)) => {
                Err(classify_acquire_error(&e.to_string(), db_name, timeout))
            }
            Err(AcquireFailure::StuckTransaction(rollback_error)) => Err(DbError::DriverError {
                driver: "sqlite".into(),
                code: None,
                message: format!(
                    "pooled connection is stuck inside a leaked transaction and ROLLBACK did \
                     not clear it{}; refusing to hand it out",
                    rollback_error
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                ),
                failed_index: None,
            }),
        }
    }
}

/// Why an acquire failed, kept apart so a stuck-transaction refusal is never
/// misclassified as pool exhaustion or a connect error.
enum AcquireFailure {
    Pool(r2d2::Error),
    StuckTransaction(Option<String>),
}

/// Compute the parent directory that must exist before SQLite can open
/// `path`, or `None` when there is nothing to create. Pure (does no IO) so
/// the path-parsing rules can be unit-tested without touching the filesystem.
///
/// Returns `None` for:
///   - in-memory databases (`:memory:`, `file::memory:?cache=shared`),
///   - bare filenames (`iii.db`) whose parent is the current directory.
///
/// Handles `file:` URI forms and strips any `?query` suffix (e.g.
/// `file:./data/iii.db?mode=rwc`) before extracting the parent.
fn parent_dir_to_create(path: &str) -> Option<std::path::PathBuf> {
    let without_scheme = path.strip_prefix("file:").unwrap_or(path);
    let file_part = without_scheme.split('?').next().unwrap_or(without_scheme);
    if file_part.is_empty() || file_part.contains(":memory:") {
        return None;
    }
    match std::path::Path::new(file_part).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
        _ => None,
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

    /// Regression (rctest5 postmortem): a caller that opened a transaction
    /// and never closed it returned its connection to the pool mid-txn;
    /// every later acquire of that connection failed `cannot start a
    /// transaction within a transaction`, starving three writer agents at
    /// once. Acquire now rolls back any leaked open transaction.
    #[tokio::test(flavor = "multi_thread")]
    async fn acquire_rolls_back_a_leaked_open_transaction() {
        let pool = SqlitePool::new(
            "sqlite::memory:",
            &PoolConfig {
                max: 1, // force reuse of the poisoned connection
                idle_timeout_ms: 30_000,
                acquire_timeout_ms: 500,
            },
        )
        .unwrap();
        let conn = pool.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            conn.with(|c| {
                c.execute_batch("BEGIN; CREATE TABLE t (n INT);").unwrap();
                assert!(!c.is_autocommit(), "transaction must be open when leaked");
            })
            // `conn` drops here still inside the transaction.
        })
        .await
        .unwrap();

        let healed = pool.acquire().await.unwrap();
        tokio::task::spawn_blocking(move || {
            healed.with(|c| {
                assert!(c.is_autocommit(), "leaked transaction must be rolled back");
                // And the connection is fully usable, including a fresh BEGIN.
                c.execute_batch("BEGIN; CREATE TABLE t2 (n INT); COMMIT;")
                    .unwrap();
            })
        })
        .await
        .unwrap();
    }

    #[test]
    fn parent_dir_to_create_skips_in_memory_forms() {
        assert_eq!(parent_dir_to_create(":memory:"), None);
        assert_eq!(parent_dir_to_create("file::memory:?cache=shared"), None);
    }

    #[test]
    fn parent_dir_to_create_skips_bare_filename() {
        // No directory component → SQLite creates the file in the CWD.
        assert_eq!(parent_dir_to_create("iii.db"), None);
        assert_eq!(parent_dir_to_create(""), None);
    }

    #[test]
    fn parent_dir_to_create_returns_nested_dir() {
        assert_eq!(
            parent_dir_to_create("./data/iii.db"),
            Some(std::path::PathBuf::from("./data"))
        );
        assert_eq!(
            parent_dir_to_create("/var/lib/iii/db.sqlite"),
            Some(std::path::PathBuf::from("/var/lib/iii"))
        );
    }

    #[test]
    fn parent_dir_to_create_handles_file_uri_and_query() {
        assert_eq!(
            parent_dir_to_create("file:./data/iii.db?mode=rwc"),
            Some(std::path::PathBuf::from("./data"))
        );
    }

    /// Regression for the registry-publish crash: a fresh boot with the
    /// default `sqlite:./data/iii.db` config (and no pre-existing `data/`
    /// dir) must succeed. r2d2 opens connections eagerly in `build()`, so
    /// before the fix `SqlitePool::new` returned
    /// `unable to open database file` and the worker died on startup, making
    /// interface collection time out. `SqlitePool::new` now creates the
    /// missing parent directory, so the pool builds and a query runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn file_pool_creates_missing_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // Point at a *nested* path that does not exist yet, mirroring the
        // default `./data/iii.db` shape (two missing levels for good measure).
        let db_path = tmp.path().join("data").join("nested").join("iii.db");
        assert!(!db_path.parent().unwrap().exists());
        let url = format!("sqlite:{}", db_path.display());

        let pool = SqlitePool::new(&url, &PoolConfig::default())
            .expect("pool should build after creating the missing parent dir");
        assert!(db_path.parent().unwrap().exists());

        let conn = pool.acquire().await.unwrap();
        let result: i64 = tokio::task::spawn_blocking(move || {
            conn.with(|c| c.query_row("SELECT 1", [], |row| row.get(0)))
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(result, 1);
    }
}
