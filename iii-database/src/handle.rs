//! Handle registry — UUID → pinned connection + SQL.
//!
//! Each entry owns a `tokio::sync::Mutex<PinnedConn>` so async drivers can
//! acquire the connection across `.await`. The outer map is a `tokio::sync::RwLock`.

use crate::error::DbError;
use chrono::{DateTime, Duration as CDuration, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HandleResponse {
    pub id: String,
    pub expires_at: DateTime<Utc>,
}

pub enum PinnedConn {
    /// SQLite is wrapped in `Option` so the SQLite driver's blocking-task
    /// shim can `.take()` ownership into `spawn_blocking` and `.replace()`
    /// it on return without needing a throwaway placeholder pool. The slot
    /// is `Some` between `prepareStatement` and the registry entry's TTL,
    /// and transiently `None` only inside an in-flight `runStatement`.
    Sqlite(Option<crate::pool::sqlite::SqliteConn>),
    Postgres(crate::pool::postgres::PgClient),
    Mysql(crate::pool::mysql::MysqlConn),
}

struct Entry {
    sql: String,
    expires_at: DateTime<Utc>,
    conn: Arc<Mutex<PinnedConn>>,
}

#[derive(Clone, Default)]
pub struct HandleRegistry {
    inner: Arc<RwLock<HashMap<String, Entry>>>,
}

impl HandleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert_sqlite(
        &self,
        sql: String,
        conn: crate::pool::sqlite::SqliteConn,
        ttl: Duration,
    ) -> HandleResponse {
        self.insert(sql, PinnedConn::Sqlite(Some(conn)), ttl).await
    }

    pub async fn insert_postgres(
        &self,
        sql: String,
        conn: crate::pool::postgres::PgClient,
        ttl: Duration,
    ) -> HandleResponse {
        self.insert(sql, PinnedConn::Postgres(conn), ttl).await
    }

    pub async fn insert_mysql(
        &self,
        sql: String,
        conn: crate::pool::mysql::MysqlConn,
        ttl: Duration,
    ) -> HandleResponse {
        self.insert(sql, PinnedConn::Mysql(conn), ttl).await
    }

    async fn insert(&self, sql: String, conn: PinnedConn, ttl: Duration) -> HandleResponse {
        let id = Uuid::new_v4().to_string();
        let expires_at =
            Utc::now() + CDuration::from_std(ttl).unwrap_or_else(|_| CDuration::seconds(3600));
        self.inner.write().await.insert(
            id.clone(),
            Entry {
                sql,
                expires_at,
                conn: Arc::new(Mutex::new(conn)),
            },
        );
        HandleResponse { id, expires_at }
    }

    pub async fn contains(&self, id: &str) -> bool {
        self.inner.read().await.contains_key(id)
    }

    pub async fn evict_expired(&self) {
        let now = Utc::now();
        self.inner.write().await.retain(|_, e| e.expires_at > now);
    }

    /// Acquire a pinned conn lock guard. Returns `STATEMENT_NOT_FOUND` if the
    /// id is unknown or expired. The caller drives `.await` against the lock.
    pub async fn lock(&self, id: &str) -> Result<(String, OwnedMutexGuard<PinnedConn>), DbError> {
        let g = self.inner.read().await;
        let entry = g.get(id).ok_or_else(|| DbError::StatementNotFound {
            handle_id: id.to_string(),
        })?;
        if entry.expires_at <= Utc::now() {
            drop(g);
            self.inner.write().await.remove(id);
            return Err(DbError::StatementNotFound {
                handle_id: id.to_string(),
            });
        }
        let sql = entry.sql.clone();
        let arc = Arc::clone(&entry.conn);
        drop(g);
        Ok((sql, arc.lock_owned().await))
    }

    pub fn spawn_evictor(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                me.evict_expired().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;
    use crate::pool::SqlitePool;
    use std::time::Duration;

    fn pool() -> SqlitePool {
        SqlitePool::new("sqlite::memory:", &PoolConfig::default()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn insert_then_lookup() {
        let reg = HandleRegistry::new();
        let p = pool();
        let conn = p.acquire().await.unwrap();
        let handle = reg
            .insert_sqlite("hot SQL".into(), conn, Duration::from_secs(60))
            .await;
        assert!(reg.contains(&handle.id).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expired_handles_get_evicted() {
        let reg = HandleRegistry::new();
        let p = pool();
        let conn = p.acquire().await.unwrap();
        let handle = reg
            .insert_sqlite("hot SQL".into(), conn, Duration::from_millis(50))
            .await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        reg.evict_expired().await;
        assert!(!reg.contains(&handle.id).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lookup_unknown_returns_statement_not_found() {
        let reg = HandleRegistry::new();
        let id = "00000000-0000-0000-0000-000000000000";
        let result = reg.lock(id).await;
        assert!(matches!(result, Err(DbError::StatementNotFound { .. })));
    }

    /// `lock()` has two STATEMENT_NOT_FOUND paths: (1) the id is missing
    /// outright, (2) the id is present but its TTL has elapsed and the
    /// background evictor has not yet run. This covers path (2) — the lazy
    /// expiry branch — and confirms it removes the stale entry as a side
    /// effect without relying on `evict_expired()`.
    #[tokio::test(flavor = "multi_thread")]
    async fn lock_on_expired_handle_returns_statement_not_found_and_evicts() {
        let reg = HandleRegistry::new();
        let p = pool();
        let conn = p.acquire().await.unwrap();
        let h = reg
            .insert_sqlite("SELECT 1".into(), conn, Duration::from_millis(1))
            .await;
        // Wait past the TTL.
        tokio::time::sleep(Duration::from_millis(20)).await;
        // Do NOT call evict_expired() — exercise the lazy path in lock().
        // `OwnedMutexGuard<PinnedConn>` doesn't impl Debug, so we can't
        // unwrap_err()/format the Result; pattern-match the error directly.
        match reg.lock(&h.id).await {
            Err(DbError::StatementNotFound { .. }) => {}
            Err(other) => panic!("expected StatementNotFound, got {other:?}"),
            Ok(_) => panic!("expected StatementNotFound, got Ok"),
        }
        assert!(
            !reg.contains(&h.id).await,
            "expired entry should be evicted by lock()"
        );
    }
}
