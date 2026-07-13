//! Queue store implementations.
//!
//! The engine builtin stores waiting job ids, delayed job ids, and DLQ entries
//! separately. This worker keeps the same behavioral model behind a smaller
//! store interface: normal queues contain both ready and delayed jobs, DLQs are
//! per topic, and file-backed mode persists the full snapshot on every
//! mutation. Retry backoff mirrors the builtin's exponential curve:
//! `backoff_ms * 2^(attempts - 1)`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

const STORE_FILE_NAME: &str = "queue_store.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub payload: Value,
    pub attempts: u32,
    pub enqueued_at_ms: u64,
    #[serde(default)]
    pub(crate) ready_at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicStats {
    pub depth: u64,
    pub dlq_depth: u64,
    pub delivered: u64,
    pub failed: u64,
}

#[async_trait]
pub trait QueueStore: Send + Sync + 'static {
    async fn enqueue(&self, topic: &str, payload: Value) -> anyhow::Result<String>;
    async fn dequeue(&self, topic: &str) -> Option<Job>;
    async fn ack(&self, topic: &str, job_id: &str);
    async fn nack(&self, topic: &str, job: Job, max_retries: u32, backoff_ms: u64);
    async fn list_topics(&self) -> Vec<String>;
    async fn topic_stats(&self, topic: &str) -> TopicStats;
    async fn dlq_topics(&self) -> Vec<(String, u64)>;
    async fn dlq_messages(&self, topic: &str, limit: u64) -> Vec<Job>;
    async fn redrive_dlq(&self, topic: &str) -> u64;
    async fn redrive_dlq_message(&self, topic: &str, job_id: &str) -> bool;
    async fn discard_dlq_message(&self, topic: &str, job_id: &str) -> bool;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoreData {
    queues: HashMap<String, VecDeque<Job>>,
    dlqs: HashMap<String, Vec<Job>>,
    stats: HashMap<String, TopicStats>,
}

#[derive(Debug)]
struct SharedStore {
    inner: Mutex<StoreData>,
    file_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct InMemoryStore {
    shared: Arc<SharedStore>,
}

#[derive(Debug, Clone)]
pub struct FileStore {
    shared: Arc<SharedStore>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SharedStore {
                inner: Mutex::new(StoreData::default()),
                file_dir: None,
            }),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FileStore {
    pub async fn open(path: impl AsRef<Path>, _save_interval_ms: u64) -> anyhow::Result<Self> {
        let dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let data = load_snapshot(&dir).await?;
        Ok(Self {
            shared: Arc::new(SharedStore {
                inner: Mutex::new(data),
                file_dir: Some(dir),
            }),
        })
    }
}

#[async_trait]
impl QueueStore for InMemoryStore {
    async fn enqueue(&self, topic: &str, payload: Value) -> anyhow::Result<String> {
        enqueue(&self.shared, topic, payload).await
    }

    async fn dequeue(&self, topic: &str) -> Option<Job> {
        dequeue(&self.shared, topic).await
    }

    async fn ack(&self, topic: &str, job_id: &str) {
        ack(&self.shared, topic, job_id).await;
    }

    async fn nack(&self, topic: &str, job: Job, max_retries: u32, backoff_ms: u64) {
        nack(&self.shared, topic, job, max_retries, backoff_ms).await;
    }

    async fn list_topics(&self) -> Vec<String> {
        list_topics(&self.shared).await
    }

    async fn topic_stats(&self, topic: &str) -> TopicStats {
        topic_stats(&self.shared, topic).await
    }

    async fn dlq_topics(&self) -> Vec<(String, u64)> {
        dlq_topics(&self.shared).await
    }

    async fn dlq_messages(&self, topic: &str, limit: u64) -> Vec<Job> {
        dlq_messages(&self.shared, topic, limit).await
    }

    async fn redrive_dlq(&self, topic: &str) -> u64 {
        redrive_dlq(&self.shared, topic).await
    }

    async fn redrive_dlq_message(&self, topic: &str, job_id: &str) -> bool {
        redrive_dlq_message(&self.shared, topic, job_id).await
    }

    async fn discard_dlq_message(&self, topic: &str, job_id: &str) -> bool {
        discard_dlq_message(&self.shared, topic, job_id).await
    }
}

#[async_trait]
impl QueueStore for FileStore {
    async fn enqueue(&self, topic: &str, payload: Value) -> anyhow::Result<String> {
        enqueue(&self.shared, topic, payload).await
    }

    async fn dequeue(&self, topic: &str) -> Option<Job> {
        dequeue(&self.shared, topic).await
    }

    async fn ack(&self, topic: &str, job_id: &str) {
        ack(&self.shared, topic, job_id).await;
    }

    async fn nack(&self, topic: &str, job: Job, max_retries: u32, backoff_ms: u64) {
        nack(&self.shared, topic, job, max_retries, backoff_ms).await;
    }

    async fn list_topics(&self) -> Vec<String> {
        list_topics(&self.shared).await
    }

    async fn topic_stats(&self, topic: &str) -> TopicStats {
        topic_stats(&self.shared, topic).await
    }

    async fn dlq_topics(&self) -> Vec<(String, u64)> {
        dlq_topics(&self.shared).await
    }

    async fn dlq_messages(&self, topic: &str, limit: u64) -> Vec<Job> {
        dlq_messages(&self.shared, topic, limit).await
    }

    async fn redrive_dlq(&self, topic: &str) -> u64 {
        redrive_dlq(&self.shared, topic).await
    }

    async fn redrive_dlq_message(&self, topic: &str, job_id: &str) -> bool {
        redrive_dlq_message(&self.shared, topic, job_id).await
    }

    async fn discard_dlq_message(&self, topic: &str, job_id: &str) -> bool {
        discard_dlq_message(&self.shared, topic, job_id).await
    }
}

async fn enqueue(shared: &SharedStore, topic: &str, payload: Value) -> anyhow::Result<String> {
    let now = now_ms();
    let job = Job {
        id: Uuid::new_v4().to_string(),
        payload,
        attempts: 0,
        enqueued_at_ms: now,
        ready_at_ms: now,
    };
    let id = job.id.clone();

    let snapshot = {
        let mut data = shared.inner.lock().await;
        data.queues
            .entry(topic.to_string())
            .or_default()
            .push_back(job);
        data.stats.entry(topic.to_string()).or_default().depth =
            data.queues.get(topic).map_or(0, |q| q.len() as u64);
        data.clone()
    };
    persist_if_needed(shared, &snapshot).await?;
    Ok(id)
}

async fn dequeue(shared: &SharedStore, topic: &str) -> Option<Job> {
    let now = now_ms();
    let result = {
        let mut data = shared.inner.lock().await;
        let queue = data.queues.get_mut(topic)?;
        let index = queue.iter().position(|job| job.ready_at_ms <= now)?;
        let job = queue.remove(index);
        if queue.is_empty() {
            data.queues.remove(topic);
        }
        let depth = data.queues.get(topic).map_or(0, |q| q.len() as u64);
        data.stats.entry(topic.to_string()).or_default().depth = depth;
        (job, data.clone())
    };

    let _ = persist_if_needed(shared, &result.1).await;
    result.0
}

async fn ack(shared: &SharedStore, topic: &str, _job_id: &str) {
    let snapshot = {
        let mut data = shared.inner.lock().await;
        data.stats.entry(topic.to_string()).or_default().delivered += 1;
        data.clone()
    };
    let _ = persist_if_needed(shared, &snapshot).await;
}

async fn nack(shared: &SharedStore, topic: &str, mut job: Job, max_retries: u32, backoff_ms: u64) {
    job.attempts = job.attempts.saturating_add(1);
    let snapshot = {
        let mut data = shared.inner.lock().await;
        if job.attempts >= max_retries {
            data.dlqs.entry(topic.to_string()).or_default().push(job);
            let dlq_depth = data.dlqs.get(topic).map_or(0, |q| q.len() as u64);
            let stats = data.stats.entry(topic.to_string()).or_default();
            stats.failed += 1;
            stats.dlq_depth = dlq_depth;
        } else {
            let delay = exponential_backoff_ms(backoff_ms, job.attempts);
            job.ready_at_ms = now_ms().saturating_add(delay);
            data.queues
                .entry(topic.to_string())
                .or_default()
                .push_back(job);
            data.stats.entry(topic.to_string()).or_default().depth =
                data.queues.get(topic).map_or(0, |q| q.len() as u64);
        }
        data.clone()
    };
    let _ = persist_if_needed(shared, &snapshot).await;
}

async fn list_topics(shared: &SharedStore) -> Vec<String> {
    let data = shared.inner.lock().await;
    let mut topics = data
        .queues
        .keys()
        .chain(data.dlqs.keys())
        .chain(data.stats.keys())
        .cloned()
        .collect::<Vec<_>>();
    topics.sort();
    topics.dedup();
    topics
}

async fn topic_stats(shared: &SharedStore, topic: &str) -> TopicStats {
    let data = shared.inner.lock().await;
    let mut stats = data.stats.get(topic).cloned().unwrap_or_default();
    stats.depth = data.queues.get(topic).map_or(0, |q| q.len() as u64);
    stats.dlq_depth = data.dlqs.get(topic).map_or(0, |q| q.len() as u64);
    stats
}

async fn dlq_topics(shared: &SharedStore) -> Vec<(String, u64)> {
    let data = shared.inner.lock().await;
    let mut topics = data
        .dlqs
        .iter()
        .filter_map(|(topic, jobs)| {
            if jobs.is_empty() {
                None
            } else {
                Some((topic.clone(), jobs.len() as u64))
            }
        })
        .collect::<Vec<_>>();
    topics.sort_by(|a, b| a.0.cmp(&b.0));
    topics
}

async fn dlq_messages(shared: &SharedStore, topic: &str, limit: u64) -> Vec<Job> {
    let data = shared.inner.lock().await;
    data.dlqs
        .get(topic)
        .map(|jobs| jobs.iter().take(limit as usize).cloned().collect())
        .unwrap_or_default()
}

async fn redrive_dlq(shared: &SharedStore, topic: &str) -> u64 {
    let snapshot_and_count = {
        let mut data = shared.inner.lock().await;
        let mut jobs = data.dlqs.remove(topic).unwrap_or_default();
        let count = jobs.len() as u64;
        for job in &mut jobs {
            job.attempts = 0;
            job.ready_at_ms = now_ms();
        }
        data.queues
            .entry(topic.to_string())
            .or_default()
            .extend(jobs);
        let depth = data.queues.get(topic).map_or(0, |q| q.len() as u64);
        let stats = data.stats.entry(topic.to_string()).or_default();
        stats.depth = depth;
        stats.dlq_depth = 0;
        (data.clone(), count)
    };
    let _ = persist_if_needed(shared, &snapshot_and_count.0).await;
    snapshot_and_count.1
}

async fn redrive_dlq_message(shared: &SharedStore, topic: &str, job_id: &str) -> bool {
    let snapshot_and_found = {
        let mut data = shared.inner.lock().await;
        let Some(dlq) = data.dlqs.get_mut(topic) else {
            return false;
        };
        let Some(index) = dlq.iter().position(|job| job.id == job_id) else {
            return false;
        };
        let mut job = dlq.remove(index);
        job.attempts = 0;
        job.ready_at_ms = now_ms();
        data.queues
            .entry(topic.to_string())
            .or_default()
            .push_back(job);
        let depth = data.queues.get(topic).map_or(0, |q| q.len() as u64);
        let dlq_depth = data.dlqs.get(topic).map_or(0, |q| q.len() as u64);
        let stats = data.stats.entry(topic.to_string()).or_default();
        stats.depth = depth;
        stats.dlq_depth = dlq_depth;
        (data.clone(), true)
    };
    let _ = persist_if_needed(shared, &snapshot_and_found.0).await;
    snapshot_and_found.1
}

async fn discard_dlq_message(shared: &SharedStore, topic: &str, job_id: &str) -> bool {
    let snapshot_and_found = {
        let mut data = shared.inner.lock().await;
        let Some(dlq) = data.dlqs.get_mut(topic) else {
            return false;
        };
        let Some(index) = dlq.iter().position(|job| job.id == job_id) else {
            return false;
        };
        dlq.remove(index);
        let dlq_depth = data.dlqs.get(topic).map_or(0, |q| q.len() as u64);
        data.stats.entry(topic.to_string()).or_default().dlq_depth = dlq_depth;
        (data.clone(), true)
    };
    let _ = persist_if_needed(shared, &snapshot_and_found.0).await;
    snapshot_and_found.1
}

async fn load_snapshot(dir: &Path) -> anyhow::Result<StoreData> {
    let path = dir.join(STORE_FILE_NAME);
    if !path.exists() {
        return Ok(StoreData::default());
    }
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn persist_if_needed(shared: &SharedStore, snapshot: &StoreData) -> anyhow::Result<()> {
    let Some(dir) = &shared.file_dir else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    let path = dir.join(STORE_FILE_NAME);
    let tmp = dir.join(format!("{STORE_FILE_NAME}.tmp"));
    let bytes = serde_json::to_vec_pretty(snapshot)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is before UNIX_EPOCH")
        .as_millis() as u64
}

fn exponential_backoff_ms(backoff_ms: u64, attempts: u32) -> u64 {
    backoff_ms.saturating_mul(2_u64.saturating_pow(attempts.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::future::Future;
    use std::pin::Pin;
    use tokio::time::{sleep, Duration};

    type StoreFuture = Pin<Box<dyn Future<Output = Arc<dyn QueueStore>> + Send>>;

    fn temp_store_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("queue_store_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn for_each_backend<F>(test: F)
    where
        F: Fn(Arc<dyn QueueStore>) -> StoreFuture,
    {
        test(Arc::new(InMemoryStore::new())).await;
        let dir = temp_store_dir();
        let store = Arc::new(FileStore::open(&dir, 5).await.unwrap());
        test(store).await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn enqueue_dequeue_ack_removes_ready_job() {
        for_each_backend(|store| {
            Box::pin(async move {
                let id = store.enqueue("demo", json!({"x": 1})).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                assert_eq!(job.id, id);
                assert_eq!(job.payload, json!({"x": 1}));
                store.ack("demo", &job.id).await;
                assert!(store.dequeue("demo").await.is_none());
                assert_eq!(store.topic_stats("demo").await.delivered, 1);
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn nack_requeues_after_exponential_backoff() {
        for_each_backend(|store| {
            Box::pin(async move {
                store.enqueue("demo", json!("job")).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                // The backoff must dwarf CI scheduling + FileStore persist
                // stalls: the store reads the wall clock, so if more than the
                // backoff elapses between nack and the next dequeue, the job
                // is already ready and the is_none assert flakes.
                store.nack("demo", job, 3, 500).await;
                assert!(store.dequeue("demo").await.is_none());
                sleep(Duration::from_millis(600)).await;
                let retry = store.dequeue("demo").await.unwrap();
                assert_eq!(retry.attempts, 1);
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn exhausted_nack_moves_to_dlq() {
        for_each_backend(|store| {
            Box::pin(async move {
                store.enqueue("demo", json!("job")).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 1, 1).await;
                assert!(store.dequeue("demo").await.is_none());
                let messages = store.dlq_messages("demo", 10).await;
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].attempts, 1);
                let stats = store.topic_stats("demo").await;
                assert_eq!(stats.dlq_depth, 1);
                assert_eq!(stats.failed, 1);
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn redrive_dlq_moves_all_back_with_attempts_reset() {
        for_each_backend(|store| {
            Box::pin(async move {
                for n in 0..2 {
                    store.enqueue("demo", json!(n)).await.unwrap();
                    let job = store.dequeue("demo").await.unwrap();
                    store.nack("demo", job, 1, 1).await;
                }
                assert_eq!(store.redrive_dlq("demo").await, 2);
                assert!(store.dlq_messages("demo", 10).await.is_empty());
                let first = store.dequeue("demo").await.unwrap();
                let second = store.dequeue("demo").await.unwrap();
                assert_eq!(first.attempts, 0);
                assert_eq!(second.attempts, 0);
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn redrive_single_dlq_message_operates_by_id() {
        for_each_backend(|store| {
            Box::pin(async move {
                store.enqueue("demo", json!("a")).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 1, 1).await;
                let id = store.dlq_messages("demo", 10).await[0].id.clone();
                assert!(!store.redrive_dlq_message("demo", "missing").await);
                assert!(store.redrive_dlq_message("demo", &id).await);
                assert_eq!(store.dequeue("demo").await.unwrap().id, id);
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn discard_single_dlq_message_operates_by_id() {
        for_each_backend(|store| {
            Box::pin(async move {
                store.enqueue("demo", json!("a")).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 1, 1).await;
                let id = store.dlq_messages("demo", 10).await[0].id.clone();
                assert!(!store.discard_dlq_message("demo", "missing").await);
                assert!(store.discard_dlq_message("demo", &id).await);
                assert!(store.dlq_messages("demo", 10).await.is_empty());
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn topics_and_stats_reflect_depths() {
        for_each_backend(|store| {
            Box::pin(async move {
                store.enqueue("demo", json!("ready")).await.unwrap();
                store.enqueue("demo", json!("still-ready")).await.unwrap();
                store.enqueue("other", json!("ready")).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 1, 1).await;
                assert_eq!(store.list_topics().await, vec!["demo", "other"]);
                assert_eq!(store.dlq_topics().await, vec![("demo".to_string(), 1)]);
                let demo = store.topic_stats("demo").await;
                assert_eq!(demo.depth, 1);
                assert_eq!(demo.dlq_depth, 1);
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn file_store_message_survives_restart() {
        let dir = temp_store_dir();
        {
            let store = FileStore::open(&dir, 5).await.unwrap();
            store
                .enqueue("demo", json!({"survives": true}))
                .await
                .unwrap();
        }
        let reopened = FileStore::open(&dir, 5).await.unwrap();
        let job = reopened.dequeue("demo").await.unwrap();
        assert_eq!(job.payload, json!({"survives": true}));
        let _ = std::fs::remove_dir_all(dir);
    }
}
