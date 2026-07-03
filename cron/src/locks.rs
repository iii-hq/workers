//! Distributed-lock gate for job fires. Parity with the builtin
//! (engine/src/workers/cron/adapters/): TTL 30s; redis uses SET NX PX with
//! key prefix `cron_lock:` and owner-checked Lua release. `local` matches the
//! builtin `kv` adapter's real semantics: process-local mutual exclusion.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::config::CronConfig;

pub const LOCK_TTL: Duration = Duration::from_millis(30_000);
const REDIS_LOCK_PREFIX: &str = "cron_lock:";
const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";

#[async_trait]
pub trait CronLock: Send + Sync + 'static {
    async fn try_acquire(&self, job_id: &str) -> bool;
    async fn release(&self, job_id: &str);
}

pub struct LocalLock {
    held: Mutex<HashMap<String, Instant>>,
    ttl: Duration,
}

impl LocalLock {
    pub fn new() -> Self {
        Self::with_ttl(LOCK_TTL)
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            held: Mutex::new(HashMap::new()),
            ttl,
        }
    }
}

impl Default for LocalLock {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CronLock for LocalLock {
    async fn try_acquire(&self, job_id: &str) -> bool {
        let mut held = self.held.lock().await;
        match held.get(job_id) {
            Some(expiry) if *expiry > Instant::now() => false,
            _ => {
                held.insert(job_id.to_string(), Instant::now() + self.ttl);
                true
            }
        }
    }

    async fn release(&self, job_id: &str) {
        self.held.lock().await.remove(job_id);
    }
}

pub struct RedisLock {
    conn: Mutex<redis::aio::ConnectionManager>,
    instance_id: String,
}

impl RedisLock {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url)?;
        let conn = redis::aio::ConnectionManager::new(client).await?;
        Ok(Self {
            conn: Mutex::new(conn),
            instance_id: uuid::Uuid::new_v4().to_string(),
        })
    }
}

#[async_trait]
impl CronLock for RedisLock {
    async fn try_acquire(&self, job_id: &str) -> bool {
        let key = format!("{REDIS_LOCK_PREFIX}{job_id}");
        let mut conn = self.conn.lock().await;
        let result: redis::RedisResult<Option<String>> = redis::cmd("SET")
            .arg(&key)
            .arg(&self.instance_id)
            .arg("NX")
            .arg("PX")
            .arg(LOCK_TTL.as_millis() as u64)
            .query_async(&mut *conn)
            .await;
        matches!(result, Ok(Some(_)))
    }

    async fn release(&self, job_id: &str) {
        const RELEASE: &str = r#"
            if redis.call('get', KEYS[1]) == ARGV[1] then
                return redis.call('del', KEYS[1])
            else
                return 0
            end"#;
        let key = format!("{REDIS_LOCK_PREFIX}{job_id}");
        let mut conn = self.conn.lock().await;
        let _: redis::RedisResult<i64> = redis::Script::new(RELEASE)
            .key(&key)
            .arg(&self.instance_id)
            .invoke_async(&mut *conn)
            .await;
    }
}

/// Build the lock backend named in the config. Unknown names error (parity
/// with the builtin's adapter registry behavior).
pub async fn build_lock(config: &CronConfig) -> anyhow::Result<Arc<dyn CronLock>> {
    match config.effective_adapter_name() {
        "local" => Ok(Arc::new(LocalLock::new())),
        "redis" => {
            let url = config
                .adapter
                .as_ref()
                .and_then(|a| a.config.as_ref())
                .and_then(|c| c.get("redis_url"))
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_REDIS_URL)
                .to_string();
            Ok(Arc::new(RedisLock::connect(&url).await?))
        }
        other => anyhow::bail!("unknown cron lock adapter '{other}' (expected 'local' or 'redis')"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_lock_acquires_and_blocks_second_owner() {
        let a = LocalLock::new();
        let b = LocalLock::new();
        assert!(a.try_acquire("job1").await);
        assert!(
            !a.try_acquire("job1").await,
            "same instance, same job: held"
        );
        assert!(
            b.try_acquire("job1").await,
            "local locks are process-local per instance"
        );
    }

    #[tokio::test]
    async fn local_lock_releases() {
        let l = LocalLock::new();
        assert!(l.try_acquire("job1").await);
        l.release("job1").await;
        assert!(l.try_acquire("job1").await);
    }

    #[tokio::test]
    async fn local_lock_expires_after_ttl() {
        let l = LocalLock::with_ttl(std::time::Duration::from_millis(20));
        assert!(l.try_acquire("job1").await);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(l.try_acquire("job1").await, "expired lock is reacquirable");
    }
}
