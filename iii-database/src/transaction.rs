//! Interactive-transaction registry.
//!
//! Holds `PinnedConn`s that are currently inside a server-side
//! `BEGIN ... COMMIT/ROLLBACK` block. Each entry pins one pool connection
//! under a UUID handle; `transactionQuery` / `transactionExecute` /
//! `commitTransaction` / `rollbackTransaction` look up the handle and
//! run SQL on the held connection.
//!
//! Timeouts are enforced by a background sweep task (`spawn_timeout_watcher`)
//! that polls every second and best-effort-rollbacks any entry whose
//! deadline has passed. The sweep takes the entry from the map *first*
//! (so concurrent handler calls fast-fail with `TRANSACTION_NOT_FOUND`),
//! then waits for any in-flight statement to release the per-conn mutex
//! before issuing `ROLLBACK` — graceful rather than ripping the connection
//! out mid-statement.

use crate::config::DriverKind;
use crate::driver;
use crate::error::DbError;
use crate::handle::PinnedConn;
use chrono::{DateTime, Duration as CDuration, Utc};
use iii_sdk::Logger;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};
use uuid::Uuid;

/// JSON wire envelope returned by `beginTransaction`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TxHandleResponse {
    pub id: String,
    pub expires_at: DateTime<Utc>,
}

struct TxEntry {
    db_name: String,
    driver: DriverKind,
    started_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
    conn: Arc<Mutex<PinnedConn>>,
}

/// Owned-guard return shape for `TxRegistry::lock`. The caller holds the
/// per-conn mutex via `guard` for the duration of one statement; the other
/// fields are pre-extracted metadata so handlers don't have to re-read the
/// map after locking.
pub struct TxLock {
    pub db_name: String,
    pub driver: DriverKind,
    pub started_at: DateTime<Utc>,
    pub guard: OwnedMutexGuard<PinnedConn>,
}

/// Return shape for `TxRegistry::take`. The caller has exclusive ownership
/// of the conn Arc; locking it and running COMMIT/ROLLBACK on the variant
/// is the caller's job.
pub struct TakenTx {
    pub db_name: String,
    pub driver: DriverKind,
    pub started_at: DateTime<Utc>,
    pub conn_arc: Arc<Mutex<PinnedConn>>,
}

/// Map of `transaction_id` (UUIDv4 string) → `TxEntry`. Cheap to clone
/// (single `Arc<RwLock<...>>`); store it in `AppState` directly.
#[derive(Clone, Default)]
pub struct TxRegistry {
    inner: Arc<RwLock<HashMap<String, TxEntry>>>,
}

impl TxRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a freshly-begun transaction. The caller has already pulled a
    /// connection from the pool and issued `BEGIN [ISOLATION LEVEL X]` on
    /// it. We just record the pinned conn + metadata and return the handle.
    pub async fn insert(
        &self,
        db_name: String,
        driver: DriverKind,
        conn: PinnedConn,
        timeout: Duration,
    ) -> TxHandleResponse {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        // Clamp pathologically-large `timeout`s to `i64::MAX` ms so the
        // chrono arithmetic doesn't panic. The handler clamps to 5 minutes
        // long before this lands.
        let deadline =
            now + CDuration::from_std(timeout).unwrap_or_else(|_| CDuration::seconds(30));
        let entry = TxEntry {
            db_name,
            driver,
            started_at: now,
            deadline,
            conn: Arc::new(Mutex::new(conn)),
        };
        self.inner.write().await.insert(id.clone(), entry);
        TxHandleResponse {
            id,
            expires_at: deadline,
        }
    }

    /// Acquire an exclusive lock on the pinned conn for query/execute.
    /// Returns `TRANSACTION_NOT_FOUND` if the id is unknown, has been
    /// finalized, or has been auto-rolled-back by the timeout watcher.
    /// The returned `OwnedMutexGuard` queues behind any concurrent
    /// statement on the same id; callers serialize naturally.
    pub async fn lock(&self, id: &str) -> Result<TxLock, DbError> {
        // Extract the Arc under a brief read lock, then await `lock_owned`
        // outside the map's read lock. Holding the map read-lock across an
        // `.await` would block all other handler lookups (begin/lock/take)
        // for the duration of any in-flight statement on this id.
        let g = self.inner.read().await;
        let entry = g.get(id).ok_or_else(|| DbError::TransactionNotFound {
            transaction_id: id.to_string(),
        })?;
        let db_name = entry.db_name.clone();
        let driver = entry.driver;
        let started_at = entry.started_at;
        let arc = Arc::clone(&entry.conn);
        drop(g);
        let guard = arc.lock_owned().await;
        Ok(TxLock {
            db_name,
            driver,
            started_at,
            guard,
        })
    }

    /// Atomically remove an entry from the registry and return the conn
    /// Arc plus metadata. Subsequent `lock()` and `take()` calls against
    /// the id return `TRANSACTION_NOT_FOUND`. Used by `commitTransaction`
    /// and `rollbackTransaction` to finalize the txn.
    pub async fn take(&self, id: &str) -> Result<TakenTx, DbError> {
        let entry =
            self.inner
                .write()
                .await
                .remove(id)
                .ok_or_else(|| DbError::TransactionNotFound {
                    transaction_id: id.to_string(),
                })?;
        Ok(TakenTx {
            db_name: entry.db_name,
            driver: entry.driver,
            started_at: entry.started_at,
            conn_arc: entry.conn,
        })
    }

    /// Spawn the timeout watcher task. Polls every 1 s, rolls back any
    /// transaction whose deadline has passed, and removes the entry. The
    /// returned `JoinHandle` is owned by `main.rs` for the lifetime of the
    /// worker process; it loops forever until the runtime is dropped.
    pub fn spawn_timeout_watcher(&self, log: Logger) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            // Drop ticks that fire while a previous sweep is still running.
            // A slow sweep (e.g. ROLLBACK of a long-running txn) would
            // otherwise queue up multiple back-to-back sweeps.
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                me.run_one_sweep(&log).await;
            }
        })
    }

    async fn run_one_sweep(&self, log: &Logger) {
        let now = Utc::now();
        // Take all expired entries in a single write-lock to keep the
        // critical section short; process them outside the lock.
        let expired: Vec<(String, TxEntry)> = {
            let mut g = self.inner.write().await;
            let ids: Vec<String> = g
                .iter()
                .filter(|(_, e)| e.deadline <= now)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| g.remove(&id).map(|e| (id, e)))
                .collect()
        };
        for (id, entry) in expired {
            let started_at = entry.started_at;
            let deadline = entry.deadline;
            let driver = entry.driver;
            let db_name = entry.db_name.clone();
            // Wait for any in-flight tx op to finish, then ROLLBACK.
            // The Arc's strong count is now 1 (we hold it; the map dropped
            // its ref), so dropping the guard at the end of the scope will
            // also drop the PinnedConn → conn returns to its pool.
            let mut guard = entry.conn.lock_owned().await;
            let result = rollback_inline(&mut guard).await;
            let now2 = Utc::now();
            let duration_ms = (now2 - started_at).num_milliseconds().max(0);
            let overshoot_ms = (now2 - deadline).num_milliseconds().max(0);
            match result {
                Ok(()) => log.warn(
                    "db_tx_timed_out",
                    Some(json!({
                        "db.system": driver_system(driver),
                        "db.name": db_name,
                        "db.transaction.id": id,
                        "duration_ms": duration_ms,
                        "deadline_overshoot_ms": overshoot_ms,
                    })),
                ),
                Err(e) => log.error(
                    "db_tx_timeout_rollback_failed",
                    Some(json!({
                        "db.system": driver_system(driver),
                        "db.name": db_name,
                        "db.transaction.id": id,
                        "duration_ms": duration_ms,
                        "error": serde_json::to_value(&e).ok(),
                    })),
                ),
            }
            drop(guard);
        }
    }

    /// Number of currently-tracked transactions. Test helper.
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

/// Map `DriverKind` to the OTel `db.system` semantic-convention value.
pub fn driver_system(d: DriverKind) -> &'static str {
    match d {
        DriverKind::Postgres => "postgres",
        DriverKind::Mysql => "mysql",
        DriverKind::Sqlite => "sqlite",
    }
}

/// Dispatch `ROLLBACK` onto whatever driver variant the locked conn is.
/// Used by the timeout sweep; explicit `rollbackTransaction` calls invoke
/// the driver helpers directly in the handler.
async fn rollback_inline(guard: &mut OwnedMutexGuard<PinnedConn>) -> Result<(), DbError> {
    match &mut **guard {
        PinnedConn::Sqlite(slot) => driver::sqlite::tx_rollback(slot).await,
        PinnedConn::Postgres(client) => driver::postgres::tx_rollback(client).await,
        PinnedConn::Mysql(conn) => driver::mysql::tx_rollback(conn).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::pool::SqlitePool;

    async fn sqlite_pinned() -> PinnedConn {
        let pool = SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap();
        let conn = pool.acquire().await.unwrap();
        PinnedConn::Sqlite(Some(conn))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn insert_then_lock_returns_metadata() {
        let reg = TxRegistry::new();
        let conn = sqlite_pinned().await;
        let h = reg
            .insert(
                "primary".into(),
                DriverKind::Sqlite,
                conn,
                Duration::from_secs(30),
            )
            .await;
        assert_eq!(h.id.len(), 36);
        let lock = reg.lock(&h.id).await.unwrap();
        assert_eq!(lock.db_name, "primary");
        assert!(matches!(lock.driver, DriverKind::Sqlite));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lock_after_take_returns_transaction_not_found() {
        // Once `take` removes an entry, subsequent `lock` lookups must
        // fast-fail with TRANSACTION_NOT_FOUND — this is what makes
        // commitTransaction/rollbackTransaction one-shot finalization.
        let reg = TxRegistry::new();
        let conn = sqlite_pinned().await;
        let h = reg
            .insert(
                "primary".into(),
                DriverKind::Sqlite,
                conn,
                Duration::from_secs(30),
            )
            .await;
        let _taken = reg.take(&h.id).await.unwrap();
        match reg.lock(&h.id).await {
            Err(DbError::TransactionNotFound { transaction_id }) => {
                assert_eq!(transaction_id, h.id);
            }
            Err(other) => panic!("expected TransactionNotFound, got {other:?}"),
            Ok(_) => panic!("expected TransactionNotFound, got Ok"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn take_twice_returns_transaction_not_found() {
        let reg = TxRegistry::new();
        let conn = sqlite_pinned().await;
        let h = reg
            .insert(
                "primary".into(),
                DriverKind::Sqlite,
                conn,
                Duration::from_secs(30),
            )
            .await;
        let _first = reg.take(&h.id).await.unwrap();
        assert!(matches!(
            reg.take(&h.id).await,
            Err(DbError::TransactionNotFound { .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lock_unknown_id_returns_transaction_not_found() {
        // `TxLock` holds an `OwnedMutexGuard` which has no `Debug` impl, so
        // we can't use `unwrap_err()` directly — pattern-match the Result.
        let reg = TxRegistry::new();
        match reg.lock("00000000-0000-0000-0000-000000000000").await {
            Err(DbError::TransactionNotFound { .. }) => {}
            Err(other) => panic!("expected TransactionNotFound, got {other:?}"),
            Ok(_) => panic!("expected TransactionNotFound, got Ok"),
        }
    }

    /// Regression: after the deadline elapses, the watcher's next sweep
    /// must (a) remove the entry from the map and (b) issue `ROLLBACK` on
    /// the pinned conn. We drive the sweep manually instead of waiting for
    /// the 1 s poll interval so the test runs in milliseconds.
    #[tokio::test(flavor = "multi_thread")]
    async fn timeout_sweep_removes_expired_entries_and_rolls_back() {
        let reg = TxRegistry::new();
        let conn = sqlite_pinned().await;
        // Open a "real" txn on the conn so ROLLBACK has something to abort.
        // BEGIN must succeed on a freshly-acquired sqlite conn — no
        // ambient transaction state from the pool.
        let mut conn = conn;
        if let PinnedConn::Sqlite(slot) = &mut conn {
            driver::sqlite::tx_begin(slot, None).await.unwrap();
        }
        // Insert with a 1ms timeout, then wait past it.
        let h = reg
            .insert(
                "primary".into(),
                DriverKind::Sqlite,
                conn,
                Duration::from_millis(1),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(reg.len().await, 1, "entry still present before sweep");

        let log = Logger::new();
        reg.run_one_sweep(&log).await;
        assert_eq!(reg.len().await, 0, "expired entry must be removed");

        // Subsequent lock fails with TRANSACTION_NOT_FOUND.
        match reg.lock(&h.id).await {
            Err(DbError::TransactionNotFound { .. }) => {}
            Err(other) => panic!("expected TransactionNotFound after sweep, got {other:?}"),
            Ok(_) => panic!("expected TransactionNotFound after sweep, got Ok"),
        }
    }

    /// Live entries (deadline still in the future) must not be touched.
    #[tokio::test(flavor = "multi_thread")]
    async fn timeout_sweep_leaves_live_entries_alone() {
        let reg = TxRegistry::new();
        let conn = sqlite_pinned().await;
        // 60-second deadline so a sub-second test can't possibly elapse it.
        let _h = reg
            .insert(
                "primary".into(),
                DriverKind::Sqlite,
                conn,
                Duration::from_secs(60),
            )
            .await;
        let log = Logger::new();
        reg.run_one_sweep(&log).await;
        assert_eq!(reg.len().await, 1, "live entry must survive sweep");
    }
}
