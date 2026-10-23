//! Distributed-lock gate for job fires. Parity with the builtin
//! (engine/src/workers/cron/adapters/): TTL 30s; redis uses SET NX PX with
//! key prefix `cron_lock:` and owner-checked Lua release. `local` matches the
//! builtin `kv` adapter's real semantics: process-local mutual exclusion.
//!
//! Each sink of a fire takes its own lock ([`LockKey`]), so instances that
//! share a backend exclude each other per binding and per observer, never
//! per expression.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::config::CronConfig;

pub const LOCK_TTL: Duration = Duration::from_millis(30_000);
const REDIS_LOCK_PREFIX: &str = "cron_lock:";
/// Observers' fires get a key space of their own: no trigger id, whatever a
/// consumer chose, can name an observer's lock.
const REDIS_OBSERVER_LOCK_PREFIX: &str = "cron_observer_lock:";
const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";

/// What a lock guards: one sink's fire.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LockKey {
    /// A `cron` trigger binding, by trigger id: taken for each fire and
    /// released when the fire ends, as it always was.
    Trigger(String),
    /// One fire of a `cron::on-tick` observer: its id (client-generated,
    /// stable across replays) and the fire's scheduled time. Never released:
    /// it expires with the TTL, so instances holding the same observer emit
    /// each period once between them however far apart they wake.
    ObserverFire {
        observer_id: String,
        scheduled_time: String,
    },
}

impl LockKey {
    /// The key in the shared backend.
    pub fn redis_key(&self) -> String {
        match self {
            Self::Trigger(trigger_id) => format!("{REDIS_LOCK_PREFIX}{trigger_id}"),
            Self::ObserverFire {
                observer_id,
                scheduled_time,
            } => format!("{REDIS_OBSERVER_LOCK_PREFIX}{observer_id}@{scheduled_time}"),
        }
    }
}

#[async_trait]
pub trait CronLock: Send + Sync + 'static {
    async fn try_acquire(&self, key: &LockKey) -> bool;
    async fn release(&self, key: &LockKey);
}

pub struct LocalLock {
    held: Mutex<Held>,
    ttl: Duration,
}

struct Held {
    keys: HashMap<LockKey, Instant>,
    /// Observer fire keys are never released: expired keys are dropped once
    /// the map reaches this size (amortized, it doubles each time).
    prune_at: usize,
}

const PRUNE_FLOOR: usize = 64;

impl LocalLock {
    pub fn new() -> Self {
        Self::with_ttl(LOCK_TTL)
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            held: Mutex::new(Held {
                keys: HashMap::new(),
                prune_at: PRUNE_FLOOR,
            }),
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
    async fn try_acquire(&self, key: &LockKey) -> bool {
        let now = Instant::now();
        let mut held = self.held.lock().await;
        if held.keys.len() >= held.prune_at {
            held.keys.retain(|_, expiry| *expiry > now);
            held.prune_at = (held.keys.len() * 2).max(PRUNE_FLOOR);
        }
        match held.keys.get(key) {
            Some(expiry) if *expiry > now => false,
            _ => {
                held.keys.insert(key.clone(), now + self.ttl);
                true
            }
        }
    }

    async fn release(&self, key: &LockKey) {
        self.held.lock().await.keys.remove(key);
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
    async fn try_acquire(&self, key: &LockKey) -> bool {
        let key = key.redis_key();
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

    async fn release(&self, key: &LockKey) {
        const RELEASE: &str = r#"
            if redis.call('get', KEYS[1]) == ARGV[1] then
                return redis.call('del', KEYS[1])
            else
                return 0
            end"#;
        let key = key.redis_key();
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

    fn job1() -> LockKey {
        LockKey::Trigger("job1".into())
    }

    fn observer_fire(observer_id: &str, scheduled_time: &str) -> LockKey {
        LockKey::ObserverFire {
            observer_id: observer_id.into(),
            scheduled_time: scheduled_time.into(),
        }
    }

    #[tokio::test]
    async fn local_lock_acquires_and_blocks_second_owner() {
        let a = LocalLock::new();
        let b = LocalLock::new();
        assert!(a.try_acquire(&job1()).await);
        assert!(
            !a.try_acquire(&job1()).await,
            "same instance, same job: held"
        );
        assert!(
            b.try_acquire(&job1()).await,
            "local locks are process-local per instance"
        );
    }

    #[tokio::test]
    async fn local_lock_releases() {
        let l = LocalLock::new();
        assert!(l.try_acquire(&job1()).await);
        l.release(&job1()).await;
        assert!(l.try_acquire(&job1()).await);
    }

    #[tokio::test]
    async fn local_lock_expires_after_ttl() {
        let l = LocalLock::with_ttl(std::time::Duration::from_millis(20));
        assert!(l.try_acquire(&job1()).await);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(l.try_acquire(&job1()).await, "expired lock is reacquirable");
    }

    /// A binding's key is exactly the one every release before
    /// cron::on-tick used, so instances mid-upgrade exclude each other.
    #[test]
    fn binding_lock_keys_keep_their_format() {
        assert_eq!(
            LockKey::Trigger("4b1f0c2e-trigger".into()).redis_key(),
            "cron_lock:4b1f0c2e-trigger"
        );
    }

    #[test]
    fn observer_lock_keys_never_collide_with_binding_keys() {
        let at = "2026-10-20T12:00:00+00:00";
        for id in ["o1", "cron_lock:o1", "cron_observer_lock:o1", ""] {
            let observer = observer_fire(id, at).redis_key();
            assert!(observer.starts_with("cron_observer_lock:"), "{observer}");
            assert!(!observer.starts_with("cron_lock:"), "{observer}");
            assert_ne!(observer, LockKey::Trigger(id.into()).redis_key());
        }
        assert_eq!(
            observer_fire("o1", at).redis_key(),
            "cron_observer_lock:o1@2026-10-20T12:00:00+00:00"
        );
        assert_ne!(
            observer_fire("o1", at),
            observer_fire("o1", "2026-10-20T12:00:01+00:00"),
            "one key per fire"
        );
    }

    #[tokio::test]
    async fn expired_observer_fire_keys_are_pruned() {
        let l = LocalLock::with_ttl(std::time::Duration::from_millis(10));
        for second in 0..PRUNE_FLOOR {
            assert!(
                l.try_acquire(&observer_fire("o1", &second.to_string()))
                    .await
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(l.try_acquire(&job1()).await);
        assert_eq!(l.held.lock().await.keys.len(), 1, "expired keys dropped");
    }
}
