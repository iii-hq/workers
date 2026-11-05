//! Queue store implementations.
//!
//! The engine builtin stores waiting job ids, delayed job ids, and DLQ entries
//! separately. This worker keeps the same behavioral model behind a smaller
//! store interface: normal queues contain both ready and delayed jobs, DLQs are
//! per topic. Retry backoff mirrors the builtin's exponential curve:
//! `backoff_ms * 2^(attempts - 1)`.
//!
//! File-backed mode appends every mutation to a journal as one JSON line, and a
//! background task folds the journal into the snapshot every `save_interval_ms`,
//! so a mutation costs the same at any queue depth. A compaction renames the
//! active journal to a segment named after the snapshot revision it leads to,
//! and `FileStore::open` replays the snapshot, the segments in numeric order
//! and the active journal, applying only entries newer than what it already
//! has. Like the snapshot rename, this survives process death but not power
//! loss: nothing is fsynced.
//!
//! An entry starts with its newline, so a line that a failed write cut short
//! ends at the next entry instead of swallowing it, and replay skips it. A
//! complete entry that replay cannot read stops `FileStore::open`, as an
//! unreadable snapshot does, and the journal stays on disk.
//!
//! Consumers never poll the store. Every mutation that can make a job
//! dequeueable (or move a queue's earliest due time) fires that queue's
//! [`QueueStore::ready_signal`], and a consumer that finds nothing ready parks
//! on that signal, plus a one-shot timer for the earliest delayed job
//! ([`QueueStore::next_ready_delay`]) when a retry backoff is pending.

use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

const STORE_FILE_NAME: &str = "queue_store.json";
const JOURNAL_FILE_NAME: &str = "queue_store.journal";

/// Held by a compaction from start to finish and by `FileStore::open` while it
/// reads the directory: the last compaction of a dropped store outlives the drop.
static DIRECTORY_CHANGES: Mutex<()> = Mutex::const_new(());

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
    /// Return an in-flight job to the front of its queue without consuming a
    /// retry attempt. Used when a function-queue consumer is replaced or
    /// shut down before it can acknowledge buffered deliveries.
    async fn requeue(&self, topic: &str, job: Job) -> anyhow::Result<()>;
    /// Wake-up signal for `topic`. The store calls `notify_waiters` on it after
    /// every mutation that can make a job dequeueable or move the queue's
    /// earliest due time: enqueue, a retrying nack, requeue, and DLQ redrive.
    ///
    /// To avoid a lost wake-up, a consumer creates (and `enable`s) its
    /// `Notified` future BEFORE the `dequeue` that comes back empty, then
    /// waits on it — a mutation landing between the emptiness check and the
    /// wait still wakes it.
    fn ready_signal(&self, topic: &str) -> Arc<Notify>;
    /// How long until the earliest waiting job of `topic` becomes
    /// dequeueable: `Some(Duration::ZERO)` when one is ready now, `None`
    /// when nothing is waiting at all. Lets an idle consumer sleep until
    /// exactly that due time (a retry backoff) instead of re-checking.
    async fn next_ready_delay(&self, topic: &str) -> Option<Duration>;
    async fn list_topics(&self) -> Vec<String>;
    async fn topic_stats(&self, topic: &str) -> TopicStats;
    async fn dlq_topics(&self) -> Vec<(String, u64)>;
    async fn dlq_messages(&self, topic: &str, limit: u64) -> Vec<Job>;
    async fn redrive_dlq(&self, topic: &str) -> u64;
    async fn redrive_dlq_message(&self, topic: &str, job_id: &str) -> bool;
    async fn discard_dlq_message(&self, topic: &str, job_id: &str) -> bool;
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct StoreData {
    #[serde(default)]
    revision: u64,
    queues: HashMap<String, VecDeque<Job>>,
    /// Jobs that have been dequeued but not yet acknowledged. Persisting this
    /// set makes the file-backed store at-least-once across worker restarts.
    #[serde(default)]
    inflight: HashMap<String, Vec<Job>>,
    dlqs: HashMap<String, Vec<Job>>,
    stats: HashMap<String, TopicStats>,
}

/// One change to the store, carrying exact values so a replay never reads the
/// clock.
#[derive(Debug, Serialize, Deserialize)]
enum Mutation {
    Enqueue {
        topic: String,
        job: Job,
    },
    Dequeue {
        topic: String,
        job_id: String,
    },
    Ack {
        topic: String,
        job_id: String,
    },
    /// `job` already carries its new `attempts` and `ready_at_ms`.
    Nack {
        topic: String,
        job: Job,
        retrying: bool,
    },
    Requeue {
        topic: String,
        job: Job,
    },
    RedriveDlq {
        topic: String,
        ready_at_ms: u64,
    },
    RedriveDlqMessage {
        topic: String,
        job_id: String,
        ready_at_ms: u64,
    },
    DiscardDlqMessage {
        topic: String,
        job_id: String,
    },
}

/// One journal line.
#[derive(Serialize, Deserialize)]
struct JournalEntry<M> {
    revision: u64,
    mutation: M,
}

#[derive(Debug)]
struct SharedStore {
    inner: Mutex<StoreData>,
    file_dir: Option<PathBuf>,
    /// Per-queue wake-up signals (see [`QueueStore::ready_signal`]). Created
    /// on demand by a waiting consumer; an entry no consumer holds any more is
    /// dropped the next time its queue is signalled.
    signals: StdMutex<HashMap<String, Arc<Notify>>>,
}

impl SharedStore {
    fn new(data: StoreData, file_dir: Option<PathBuf>) -> Self {
        Self {
            inner: Mutex::new(data),
            file_dir,
            signals: StdMutex::new(HashMap::new()),
        }
    }

    fn ready_signal(&self, topic: &str) -> Arc<Notify> {
        self.signals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(topic.to_string())
            .or_default()
            .clone()
    }

    /// Wake every consumer parked on `topic`. Called after the mutation is
    /// visible under the store lock, so a woken consumer's next `dequeue`
    /// observes it.
    fn notify_ready(&self, topic: &str) {
        let mut signals = self.signals.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(signal) = signals.get(topic) {
            if Arc::strong_count(signal) == 1 {
                // Only the map holds it: no consumer is attached any more.
                signals.remove(topic);
            } else {
                signal.notify_waiters();
            }
        }
    }
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
            shared: Arc::new(SharedStore::new(StoreData::default(), None)),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FileStore {
    pub async fn open(path: impl AsRef<Path>, save_interval_ms: u64) -> anyhow::Result<Self> {
        let dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let data = {
            let _directory = DIRECTORY_CHANGES.lock().await;
            load_store(&dir)?
        };
        let snapshot_revision = data.revision;
        let shared = Arc::new(SharedStore::new(data, Some(dir.clone())));
        spawn_compactor(
            Arc::downgrade(&shared),
            dir,
            save_interval_ms,
            snapshot_revision,
        );
        Ok(Self { shared })
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

    async fn requeue(&self, topic: &str, job: Job) -> anyhow::Result<()> {
        requeue(&self.shared, topic, job).await
    }

    fn ready_signal(&self, topic: &str) -> Arc<Notify> {
        self.shared.ready_signal(topic)
    }

    async fn next_ready_delay(&self, topic: &str) -> Option<Duration> {
        next_ready_delay(&self.shared, topic).await
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

    async fn requeue(&self, topic: &str, job: Job) -> anyhow::Result<()> {
        requeue(&self.shared, topic, job).await
    }

    fn ready_signal(&self, topic: &str) -> Arc<Notify> {
        self.shared.ready_signal(topic)
    }

    async fn next_ready_delay(&self, topic: &str) -> Option<Duration> {
        next_ready_delay(&self.shared, topic).await
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
    let mutation = Mutation::Enqueue {
        topic: topic.to_string(),
        job,
    };

    // Nothing applies until the journal line is written, so a rejected publish
    // cannot leave a runnable in-memory "ghost" job.
    let mut data = shared.inner.lock().await;
    let revision = data.revision.saturating_add(1);
    append_journal(shared, revision, &mutation)?;
    apply(&mut data, &mutation);
    data.revision = revision;
    drop(data);
    shared.notify_ready(topic);
    Ok(id)
}

async fn dequeue(shared: &SharedStore, topic: &str) -> Option<Job> {
    let now = now_ms();
    let mut data = shared.inner.lock().await;
    let job = data
        .queues
        .get(topic)?
        .iter()
        .find(|job| job.ready_at_ms <= now)?
        .clone();
    let mutation = Mutation::Dequeue {
        topic: topic.to_string(),
        job_id: job.id.clone(),
    };
    commit_logged(shared, &mut data, &mutation);
    Some(job)
}

async fn ack(shared: &SharedStore, topic: &str, job_id: &str) {
    let mutation = Mutation::Ack {
        topic: topic.to_string(),
        job_id: job_id.to_string(),
    };
    commit_logged(shared, &mut *shared.inner.lock().await, &mutation);
}

async fn nack(shared: &SharedStore, topic: &str, mut job: Job, max_retries: u32, backoff_ms: u64) {
    job.attempts = job.attempts.saturating_add(1);
    let retrying = job.attempts < max_retries;
    if retrying {
        let delay = exponential_backoff_ms(backoff_ms, job.attempts);
        job.ready_at_ms = now_ms().saturating_add(delay);
    }
    let mutation = Mutation::Nack {
        topic: topic.to_string(),
        job,
        retrying,
    };
    commit_logged(shared, &mut *shared.inner.lock().await, &mutation);
    if retrying {
        // The retry is delayed, but waiters must still learn the queue's new
        // earliest due time so they can arm a timer for it.
        shared.notify_ready(topic);
    }
}

async fn requeue(shared: &SharedStore, topic: &str, mut job: Job) -> anyhow::Result<()> {
    job.ready_at_ms = now_ms();
    let mutation = Mutation::Requeue {
        topic: topic.to_string(),
        job,
    };
    let committed = commit(shared, &mut *shared.inner.lock().await, &mutation);
    shared.notify_ready(topic);
    committed
}

async fn next_ready_delay(shared: &SharedStore, topic: &str) -> Option<Duration> {
    let data = shared.inner.lock().await;
    let earliest = data
        .queues
        .get(topic)?
        .iter()
        .map(|job| job.ready_at_ms)
        .min()?;
    Some(Duration::from_millis(earliest.saturating_sub(now_ms())))
}

fn remove_inflight(data: &mut StoreData, topic: &str, job_id: &str) {
    let Some(jobs) = data.inflight.get_mut(topic) else {
        return;
    };
    if let Some(index) = jobs.iter().position(|job| job.id == job_id) {
        jobs.remove(index);
    }
    if jobs.is_empty() {
        data.inflight.remove(topic);
    }
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
    let mutation = Mutation::RedriveDlq {
        topic: topic.to_string(),
        ready_at_ms: now_ms(),
    };
    let count = {
        let mut data = shared.inner.lock().await;
        let count = data.dlqs.get(topic).map_or(0, |jobs| jobs.len() as u64);
        commit_logged(shared, &mut data, &mutation);
        count
    };
    if count > 0 {
        shared.notify_ready(topic);
    }
    count
}

async fn redrive_dlq_message(shared: &SharedStore, topic: &str, job_id: &str) -> bool {
    let mutation = Mutation::RedriveDlqMessage {
        topic: topic.to_string(),
        job_id: job_id.to_string(),
        ready_at_ms: now_ms(),
    };
    {
        let mut data = shared.inner.lock().await;
        if !dlq_contains(&data, topic, job_id) {
            return false;
        }
        commit_logged(shared, &mut data, &mutation);
    }
    shared.notify_ready(topic);
    true
}

async fn discard_dlq_message(shared: &SharedStore, topic: &str, job_id: &str) -> bool {
    let mut data = shared.inner.lock().await;
    if !dlq_contains(&data, topic, job_id) {
        return false;
    }
    let mutation = Mutation::DiscardDlqMessage {
        topic: topic.to_string(),
        job_id: job_id.to_string(),
    };
    commit_logged(shared, &mut data, &mutation);
    true
}

fn dlq_contains(data: &StoreData, topic: &str, job_id: &str) -> bool {
    data.dlqs
        .get(topic)
        .is_some_and(|jobs| jobs.iter().any(|job| job.id == job_id))
}

/// The only code that changes a [`StoreData`]: the live path and replay share
/// it. A missing target is not an error, because a failed append can leave a
/// gap in the journal.
fn apply(data: &mut StoreData, mutation: &Mutation) {
    match mutation {
        Mutation::Enqueue { topic, job } => {
            data.queues
                .entry(topic.clone())
                .or_default()
                .push_back(job.clone());
            sync_depths(data, topic);
        }
        Mutation::Dequeue { topic, job_id } => {
            let Some(queue) = data.queues.get_mut(topic) else {
                return;
            };
            let Some(job) = queue
                .iter()
                .position(|job| job.id == *job_id)
                .and_then(|index| queue.remove(index))
            else {
                return;
            };
            if queue.is_empty() {
                data.queues.remove(topic);
            }
            data.inflight.entry(topic.clone()).or_default().push(job);
            sync_depths(data, topic);
        }
        Mutation::Ack { topic, job_id } => {
            remove_inflight(data, topic, job_id);
            data.stats.entry(topic.clone()).or_default().delivered += 1;
        }
        Mutation::Nack {
            topic,
            job,
            retrying,
        } => {
            remove_inflight(data, topic, &job.id);
            if *retrying {
                data.queues
                    .entry(topic.clone())
                    .or_default()
                    .push_back(job.clone());
            } else {
                data.dlqs
                    .entry(topic.clone())
                    .or_default()
                    .push(job.clone());
                data.stats.entry(topic.clone()).or_default().failed += 1;
            }
            sync_depths(data, topic);
        }
        Mutation::Requeue { topic, job } => {
            remove_inflight(data, topic, &job.id);
            data.queues
                .entry(topic.clone())
                .or_default()
                .push_front(job.clone());
            sync_depths(data, topic);
        }
        Mutation::RedriveDlq { topic, ready_at_ms } => {
            let mut jobs = data.dlqs.remove(topic).unwrap_or_default();
            for job in &mut jobs {
                job.attempts = 0;
                job.ready_at_ms = *ready_at_ms;
            }
            data.queues.entry(topic.clone()).or_default().extend(jobs);
            sync_depths(data, topic);
        }
        Mutation::RedriveDlqMessage {
            topic,
            job_id,
            ready_at_ms,
        } => {
            let Some(mut job) = take_dlq_message(data, topic, job_id) else {
                return;
            };
            job.attempts = 0;
            job.ready_at_ms = *ready_at_ms;
            data.queues.entry(topic.clone()).or_default().push_back(job);
            sync_depths(data, topic);
        }
        Mutation::DiscardDlqMessage { topic, job_id } => {
            take_dlq_message(data, topic, job_id);
            sync_depths(data, topic);
        }
    }
}

fn take_dlq_message(data: &mut StoreData, topic: &str, job_id: &str) -> Option<Job> {
    let jobs = data.dlqs.get_mut(topic)?;
    let index = jobs.iter().position(|job| job.id == job_id)?;
    Some(jobs.remove(index))
}

fn sync_depths(data: &mut StoreData, topic: &str) {
    let depth = data.queues.get(topic).map_or(0, |queue| queue.len() as u64);
    let dlq_depth = data.dlqs.get(topic).map_or(0, |jobs| jobs.len() as u64);
    let stats = data.stats.entry(topic.to_string()).or_default();
    stats.depth = depth;
    stats.dlq_depth = dlq_depth;
}

/// Journal `mutation` and apply it. It takes effect even when the append fails;
/// the error tells the caller it may not survive a restart.
fn commit(shared: &SharedStore, data: &mut StoreData, mutation: &Mutation) -> anyhow::Result<()> {
    let revision = data.revision.saturating_add(1);
    let appended = append_journal(shared, revision, mutation);
    apply(data, mutation);
    data.revision = revision;
    appended
}

fn commit_logged(shared: &SharedStore, data: &mut StoreData, mutation: &Mutation) {
    if let Err(error) = commit(shared, data, mutation) {
        tracing::warn!(error = %error, "queue journal append failed; the change may not survive a restart");
    }
}

fn append_journal(shared: &SharedStore, revision: u64, mutation: &Mutation) -> anyhow::Result<()> {
    let Some(dir) = &shared.file_dir else {
        return Ok(());
    };
    let mut line = vec![b'\n'];
    serde_json::to_writer(&mut line, &JournalEntry { revision, mutation })?;
    let path = dir.join(JOURNAL_FILE_NAME);
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    // Opened per append: a held handle would keep writing into a directory that
    // has been replaced instead of failing.
    let mut file = match options.open(&path) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
            options.open(&path)?
        }
        file => file?,
    };
    file.write_all(&line)?;
    Ok(())
}

fn load_store(dir: &Path) -> anyhow::Result<StoreData> {
    let snapshot = dir.join(STORE_FILE_NAME);
    let has_snapshot = snapshot.exists();
    let mut data: StoreData = if has_snapshot {
        serde_json::from_slice(&std::fs::read(snapshot)?)?
    } else {
        StoreData::default()
    };
    let mut journals = journal_segments(dir)?
        .into_iter()
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    let active = dir.join(JOURNAL_FILE_NAME);
    if active.exists() {
        journals.push(active);
    }
    for journal in &journals {
        replay_journal(&mut data, journal)?;
    }
    let restores_jobs = !data.inflight.is_empty();
    // A previous process can only leave entries in `inflight` by terminating
    // before ack/nack. Restore them ahead of waiting jobs so they are delivered
    // again in their original dequeue order.
    for (topic, mut jobs) in std::mem::take(&mut data.inflight) {
        let queue = data.queues.entry(topic.clone()).or_default();
        while let Some(mut job) = jobs.pop() {
            job.ready_at_ms = now_ms();
            queue.push_front(job);
        }
        data.stats.entry(topic).or_default().depth = queue.len() as u64;
    }
    // The next replay starts from this snapshot, so it must hold the restored
    // order, and only then can the journals it covers go.
    if !has_snapshot || !journals.is_empty() || restores_jobs {
        write_snapshot(dir, &data)?;
        for journal in &journals {
            std::fs::remove_file(journal)?;
        }
    }
    Ok(data)
}

fn replay_journal(data: &mut StoreData, path: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(path)?;
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let entry: JournalEntry<Mutation> = match serde_json::from_slice(line) {
            Ok(entry) => entry,
            Err(error) if error.is_eof() => {
                tracing::warn!(path = %path.display(), "ignoring a queue journal line cut short by a failed write or a crash");
                continue;
            }
            Err(error) => anyhow::bail!(
                "unreadable entry in queue journal {}: {error}",
                path.display()
            ),
        };
        if entry.revision > data.revision {
            apply(data, &entry.mutation);
            data.revision = entry.revision;
        }
    }
    Ok(())
}

/// Rotated journal segments by revision, oldest first.
fn journal_segments(dir: &Path) -> std::io::Result<Vec<(u64, PathBuf)>> {
    let prefix = format!("{JOURNAL_FILE_NAME}.");
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let revision = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|revision| revision.parse().ok());
        if let Some(revision) = revision {
            segments.push((revision, entry.path()));
        }
    }
    segments.sort();
    Ok(segments)
}

fn rotate_journal(dir: &Path, revision: u64) -> std::io::Result<()> {
    let segment = dir.join(format!("{JOURNAL_FILE_NAME}.{revision}"));
    // A retry at an unchanged revision: the segment holds the entries, and the
    // active file can only hold what a rejected publish left.
    if segment.exists() {
        return Ok(());
    }
    match std::fs::rename(dir.join(JOURNAL_FILE_NAME), segment) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_snapshot(dir: &Path, data: &StoreData) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{STORE_FILE_NAME}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(data)?)?;
    std::fs::rename(tmp, dir.join(STORE_FILE_NAME))?;
    Ok(())
}

/// Fold the journal into the snapshot every `save_interval_ms` until the store
/// is dropped.
fn spawn_compactor(
    store: Weak<SharedStore>,
    dir: PathBuf,
    save_interval_ms: u64,
    mut snapshot_revision: u64,
) {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_millis(save_interval_ms.max(1)));
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // The first tick completes immediately; compaction starts one interval in.
        ticks.tick().await;
        loop {
            ticks.tick().await;
            let _directory = DIRECTORY_CHANGES.lock().await;
            let Some(shared) = store.upgrade() else {
                return;
            };
            match compact(&shared, &dir, snapshot_revision).await {
                Ok(revision) => snapshot_revision = revision,
                Err(error) => {
                    tracing::warn!(error = %error, "queue store compaction failed; keeping the journal and retrying on the next tick");
                }
            }
        }
    });
}

/// Returns the revision the snapshot on disk now holds. The rotation and the
/// clone happen under the store lock so the segment holds exactly the entries
/// up to that revision; nothing else in here blocks mutations.
async fn compact(shared: &SharedStore, dir: &Path, snapshot_revision: u64) -> anyhow::Result<u64> {
    let data = {
        let data = shared.inner.lock().await;
        if data.revision == snapshot_revision {
            return Ok(snapshot_revision);
        }
        rotate_journal(dir, data.revision)?;
        data.clone()
    };
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        write_snapshot(&dir, &data)?;
        for (revision, path) in journal_segments(&dir)? {
            if revision <= data.revision {
                std::fs::remove_file(path)?;
            }
        }
        Ok(data.revision)
    })
    .await?
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

    /// Every mutation that can make a job dequeueable — or move the queue's
    /// earliest due time — wakes consumers parked on the queue's signal.
    #[tokio::test]
    async fn ready_signal_fires_on_every_mutation_that_can_make_a_job_ready() {
        use futures::FutureExt;

        async fn assert_signals<F: Future>(store: &Arc<dyn QueueStore>, what: &str, op: F) {
            let signal = store.ready_signal("demo");
            let notified = signal.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            op.await;
            assert!(
                notified.as_mut().now_or_never().is_some(),
                "{what} must wake consumers parked on the queue"
            );
        }

        for_each_backend(|store| {
            Box::pin(async move {
                assert_signals(&store, "enqueue", async {
                    store.enqueue("demo", json!("a")).await.unwrap();
                })
                .await;
                let job = store.dequeue("demo").await.unwrap();
                assert_signals(&store, "retrying nack", store.nack("demo", job, 3, 10_000)).await;

                // The backed-off "a" is skipped; "b" is the ready job.
                store.enqueue("demo", json!("b")).await.unwrap();
                let job = store.dequeue("demo").await.unwrap();
                assert_eq!(job.payload, json!("b"));
                assert_signals(&store, "requeue", async {
                    store.requeue("demo", job).await.unwrap();
                })
                .await;

                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 1, 1).await;
                assert_signals(&store, "dlq redrive", async {
                    assert_eq!(store.redrive_dlq("demo").await, 1);
                })
                .await;

                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 1, 1).await;
                let id = store.dlq_messages("demo", 1).await[0].id.clone();
                assert_signals(&store, "single-message redrive", async {
                    assert!(store.redrive_dlq_message("demo", &id).await);
                })
                .await;
                store
            })
        })
        .await;
    }

    #[tokio::test]
    async fn next_ready_delay_reports_the_earliest_due_time() {
        for_each_backend(|store| {
            Box::pin(async move {
                assert_eq!(store.next_ready_delay("demo").await, None);
                store.enqueue("demo", json!("job")).await.unwrap();
                assert_eq!(store.next_ready_delay("demo").await, Some(Duration::ZERO));

                let job = store.dequeue("demo").await.unwrap();
                store.nack("demo", job, 3, 10_000).await;
                let due_in = store.next_ready_delay("demo").await.unwrap();
                assert!(
                    due_in > Duration::from_secs(9) && due_in <= Duration::from_secs(10),
                    "a backed-off retry is due after its backoff, got {due_in:?}"
                );

                store.enqueue("demo", json!("ready")).await.unwrap();
                assert_eq!(
                    store.next_ready_delay("demo").await,
                    Some(Duration::ZERO),
                    "a ready job behind a delayed one is due now"
                );
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

    #[tokio::test]
    async fn file_store_restores_unacknowledged_jobs_ahead_of_waiting_jobs() {
        let dir = temp_store_dir();
        let first_id = {
            let store = FileStore::open(&dir, 5).await.unwrap();
            let first_id = store.enqueue("demo", json!("first")).await.unwrap();
            store.enqueue("demo", json!("second")).await.unwrap();
            let delivery = store.dequeue("demo").await.unwrap();
            assert_eq!(delivery.id, first_id);
            first_id
        };

        let reopened = FileStore::open(&dir, 5).await.unwrap();
        let first = reopened.dequeue("demo").await.unwrap();
        let second = reopened.dequeue("demo").await.unwrap();
        assert_eq!(first.id, first_id);
        assert_eq!(first.payload, json!("first"));
        assert_eq!(second.payload, json!("second"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn file_store_failed_enqueue_does_not_leave_runnable_job() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, 5).await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::write(&dir, b"not a directory").unwrap();

        let result = store.enqueue("demo", json!("ghost")).await;
        assert!(result.is_err());
        assert!(store.dequeue("demo").await.is_none());
        assert_eq!(store.topic_stats("demo").await.depth, 0);

        let _ = std::fs::remove_file(dir);
    }

    const HUGE_INTERVAL_MS: u64 = 3_600_000;
    /// What a write stopped mid-line leaves in the journal.
    const CUT_SHORT_ENTRY: &[u8] =
        b"\n{\"revision\":2,\"mutation\":{\"Enqueue\":{\"topic\":\"demo\",\"jo";

    async fn state(store: &FileStore) -> StoreData {
        store.shared.inner.lock().await.clone()
    }

    fn snapshot_on_disk(dir: &Path) -> StoreData {
        serde_json::from_slice(&std::fs::read(dir.join(STORE_FILE_NAME)).unwrap()).unwrap()
    }

    async fn payloads_in_order(store: &FileStore, topic: &str) -> Vec<Value> {
        let mut payloads = Vec::new();
        while let Some(job) = store.dequeue(topic).await {
            payloads.push(job.payload);
        }
        payloads
    }

    #[tokio::test]
    async fn every_mutation_replayed_from_the_journal_alone_matches_the_live_store() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        for payload in ["acked", "retried", "requeued", "one", "discarded", "all"] {
            store.enqueue("demo", json!(payload)).await.unwrap();
        }
        store.enqueue("other", json!("untouched")).await.unwrap();

        let acked = store.dequeue("demo").await.unwrap();
        store.ack("demo", &acked.id).await;
        let retried = store.dequeue("demo").await.unwrap();
        store.nack("demo", retried, 3, 60_000).await;
        let requeued = store.dequeue("demo").await.unwrap();
        store.requeue("demo", requeued).await.unwrap();
        let mut dead = Vec::new();
        for _ in 0..4 {
            let job = store.dequeue("demo").await.unwrap();
            dead.push(job.id.clone());
            store.nack("demo", job, 1, 1).await;
        }
        assert!(store.redrive_dlq_message("demo", &dead[1]).await);
        assert!(store.discard_dlq_message("demo", &dead[2]).await);
        assert_eq!(store.redrive_dlq("demo").await, 2);

        let live = state(&store).await;
        assert!(
            live.inflight.is_empty(),
            "restoration must have nothing to do"
        );
        let journal = std::fs::read_to_string(dir.join(JOURNAL_FILE_NAME)).unwrap();
        for variant in [
            "Enqueue",
            "Dequeue",
            "Ack",
            "Nack",
            "Requeue",
            "RedriveDlq",
            "RedriveDlqMessage",
            "DiscardDlqMessage",
        ] {
            assert!(journal.contains(&format!("\"{variant}\"")), "{variant}");
        }
        assert_eq!(snapshot_on_disk(&dir).revision, 0, "nothing was compacted");
        drop(store);

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(state(&reopened).await, live);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn torn_final_journal_line_is_ignored_and_the_store_stays_usable() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        store.enqueue("demo", json!("first")).await.unwrap();
        store.enqueue("demo", json!("second")).await.unwrap();
        drop(store);
        let mut journal = OpenOptions::new()
            .append(true)
            .open(dir.join(JOURNAL_FILE_NAME))
            .unwrap();
        journal.write_all(CUT_SHORT_ENTRY).unwrap();
        drop(journal);

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(reopened.topic_stats("demo").await.depth, 2);
        reopened.enqueue("demo", json!("third")).await.unwrap();
        drop(reopened);

        let again = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(
            payloads_in_order(&again, "demo").await,
            vec![json!("first"), json!("second"), json!("third")],
            "an entry written after the torn line must survive another reopen"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn current_format_snapshot_without_a_journal_loads_unchanged() {
        let dir = temp_store_dir();
        let fixture = r#"{
  "revision": 7,
  "queues": {
    "demo": [
      { "id": "waiting", "payload": { "n": 1 }, "attempts": 0, "enqueued_at_ms": 1, "ready_at_ms": 1 }
    ]
  },
  "inflight": {
    "demo": [
      { "id": "unacked", "payload": { "n": 0 }, "attempts": 2, "enqueued_at_ms": 1, "ready_at_ms": 1 }
    ]
  },
  "dlqs": {
    "demo": [
      { "id": "dead", "payload": "x", "attempts": 3, "enqueued_at_ms": 1, "ready_at_ms": 1 }
    ]
  },
  "stats": {
    "demo": { "depth": 1, "dlq_depth": 1, "delivered": 4, "failed": 1 }
  }
}"#;
        std::fs::write(dir.join(STORE_FILE_NAME), fixture).unwrap();

        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(state(&store).await.revision, 7);
        assert_eq!(snapshot_on_disk(&dir), state(&store).await);
        assert_eq!(store.dlq_messages("demo", 10).await[0].id, "dead");
        let stats = store.topic_stats("demo").await;
        assert_eq!((stats.delivered, stats.failed), (4, 1));
        let unacked = store.dequeue("demo").await.unwrap();
        assert_eq!((unacked.id.as_str(), unacked.attempts), ("unacked", 2));
        assert_eq!(store.dequeue("demo").await.unwrap().id, "waiting");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn segments_and_active_journal_replay_in_numeric_order_after_a_crash_before_the_snapshot()
    {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        for n in 0..9 {
            store.enqueue("demo", json!(n)).await.unwrap();
        }
        rotate_journal(&dir, 9).unwrap();
        store.enqueue("demo", json!(9)).await.unwrap();
        rotate_journal(&dir, 10).unwrap();
        store.enqueue("demo", json!(10)).await.unwrap();
        drop(store);

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(state(&reopened).await.revision, 11);
        assert_eq!(
            payloads_in_order(&reopened, "demo").await,
            (0..=10).map(|n| json!(n)).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn entries_at_or_below_the_snapshot_revision_are_not_applied_twice() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        store.enqueue("demo", json!("in snapshot")).await.unwrap();
        // A crash between the snapshot rename and the journal delete.
        write_snapshot(&dir, &state(&store).await).unwrap();
        store.enqueue("demo", json!("journal only")).await.unwrap();
        drop(store);

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(
            payloads_in_order(&reopened, "demo").await,
            vec![json!("in snapshot"), json!("journal only")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A mutation must not cost a snapshot rewrite: that made the cost grow with
    /// depth until a busy queue could never catch up.
    #[tokio::test]
    async fn enqueues_leave_the_snapshot_untouched_and_grow_only_the_journal() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        let snapshot = std::fs::read(dir.join(STORE_FILE_NAME)).unwrap();
        for n in 0..1000 {
            store.enqueue("demo", json!(n)).await.unwrap();
        }
        assert_eq!(std::fs::read(dir.join(STORE_FILE_NAME)).unwrap(), snapshot);
        let journal = std::fs::read_to_string(dir.join(JOURNAL_FILE_NAME)).unwrap();
        assert_eq!(journal.trim_start().lines().count(), 1000);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn compaction_folds_the_journal_into_the_snapshot_and_skips_when_nothing_moved() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        store.enqueue("demo", json!("job")).await.unwrap();
        store.dequeue("demo").await.unwrap();

        let revision = compact(&store.shared, &dir, 0).await.unwrap();
        assert_eq!(revision, 2);
        assert_eq!(snapshot_on_disk(&dir), state(&store).await);
        assert!(!dir.join(JOURNAL_FILE_NAME).exists());
        assert!(journal_segments(&dir).unwrap().is_empty());

        std::fs::remove_file(dir.join(STORE_FILE_NAME)).unwrap();
        assert_eq!(compact(&store.shared, &dir, 2).await.unwrap(), 2);
        assert!(!dir.join(STORE_FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn failed_snapshot_keeps_the_segment_and_the_next_compaction_recovers() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        store.enqueue("demo", json!("kept")).await.unwrap();
        let blocker = dir.join(format!("{STORE_FILE_NAME}.tmp"));
        std::fs::create_dir(&blocker).unwrap();

        assert!(compact(&store.shared, &dir, 0).await.is_err());
        assert_eq!(snapshot_on_disk(&dir).revision, 0);
        assert_eq!(journal_segments(&dir).unwrap().len(), 1);

        std::fs::remove_dir(&blocker).unwrap();
        assert_eq!(compact(&store.shared, &dir, 0).await.unwrap(), 1);
        assert!(journal_segments(&dir).unwrap().is_empty());
        drop(store);
        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(
            payloads_in_order(&reopened, "demo").await,
            vec![json!("kept")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// At an unchanged revision the active journal holds only what a rejected
    /// publish left, and renaming that over the segment a failed snapshot kept
    /// would destroy the only copy of those entries.
    #[tokio::test]
    async fn retried_compaction_keeps_the_segment_a_failed_snapshot_left() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        store.enqueue("demo", json!("kept")).await.unwrap();
        let blocker = dir.join(format!("{STORE_FILE_NAME}.tmp"));
        std::fs::create_dir(&blocker).unwrap();

        assert!(compact(&store.shared, &dir, 0).await.is_err());
        std::fs::write(dir.join(JOURNAL_FILE_NAME), CUT_SHORT_ENTRY).unwrap();
        assert!(compact(&store.shared, &dir, 0).await.is_err());
        store.enqueue("demo", json!("later")).await.unwrap();
        drop(store);
        std::fs::remove_dir(&blocker).unwrap();

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(
            payloads_in_order(&reopened, "demo").await,
            vec![json!("kept"), json!("later")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A full disk can stop a write mid-line. That publish was rejected, but the
    /// ones acknowledged after it must not be appended onto its partial line.
    #[tokio::test]
    async fn acknowledged_entries_after_a_write_cut_short_survive_a_restart() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        store.enqueue("demo", json!("before")).await.unwrap();
        let mut journal = OpenOptions::new()
            .append(true)
            .open(dir.join(JOURNAL_FILE_NAME))
            .unwrap();
        journal.write_all(CUT_SHORT_ENTRY).unwrap();
        drop(journal);
        store.enqueue("demo", json!("after")).await.unwrap();
        store.enqueue("demo", json!("last")).await.unwrap();
        drop(store);

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(
            payloads_in_order(&reopened, "demo").await,
            vec![json!("before"), json!("after"), json!("last")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Replay skips only a line that ends early, so a write cut short at any byte
    /// has to read that way or it would refuse the whole store.
    #[test]
    fn every_prefix_of_a_journal_entry_reads_as_cut_short() {
        let mutation = Mutation::Nack {
            topic: "demo".to_string(),
            job: Job {
                id: "job".to_string(),
                payload: json!({"text": "\"\u{e9}\n\u{1}", "values": [-1.5e3, 0.5, true, null]}),
                attempts: 1,
                enqueued_at_ms: 2,
                ready_at_ms: 3,
            },
            retrying: true,
        };
        let line = serde_json::to_vec(&JournalEntry {
            revision: 1,
            mutation: &mutation,
        })
        .unwrap();
        assert!(serde_json::from_slice::<JournalEntry<Mutation>>(&line).is_ok());
        for end in 1..line.len() {
            let error = serde_json::from_slice::<JournalEntry<Mutation>>(&line[..end])
                .err()
                .unwrap_or_else(|| panic!("a prefix of {end} bytes parsed"));
            assert!(error.is_eof(), "{end} bytes: {error}");
        }
    }

    /// A complete entry that replay cannot read is not a torn tail: skipping it
    /// would lose an acknowledged job, and deleting the journal would hide that.
    #[tokio::test]
    async fn a_complete_entry_replay_cannot_read_refuses_to_open_and_keeps_the_journal() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        let mut beyond_the_parser_limit = json!(1);
        for _ in 0..128 {
            beyond_the_parser_limit = json!([beyond_the_parser_limit]);
        }
        store.enqueue("demo", json!("before")).await.unwrap();
        store
            .enqueue("demo", beyond_the_parser_limit)
            .await
            .unwrap();
        store.enqueue("demo", json!("after")).await.unwrap();
        drop(store);
        let journal = std::fs::read(dir.join(JOURNAL_FILE_NAME)).unwrap();

        assert!(FileStore::open(&dir, HUGE_INTERVAL_MS).await.is_err());
        assert_eq!(std::fs::read(dir.join(JOURNAL_FILE_NAME)).unwrap(), journal);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Replay cannot rebuild an order that lives only in memory: a dequeue of a
    /// restored job is a no-op on the snapshot, so a requeue would jump the queue.
    #[tokio::test]
    async fn restored_jobs_keep_their_order_when_one_is_requeued_before_a_restart() {
        let dir = temp_store_dir();
        let job = |id: &str| json!({"id": id, "payload": id, "attempts": 0, "enqueued_at_ms": 1, "ready_at_ms": 1});
        let snapshot = json!({
            "revision": 7,
            "queues": {"demo": [job("waiting")]},
            "inflight": {"demo": [job("r1"), job("r2"), job("r3")]},
            "dlqs": {},
            "stats": {},
        });
        std::fs::write(dir.join(STORE_FILE_NAME), snapshot.to_string()).unwrap();

        let store = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        let first = store.dequeue("demo").await.unwrap();
        assert_eq!(first.id, "r1");
        store.requeue("demo", first).await.unwrap();
        drop(store);

        let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
        assert_eq!(
            payloads_in_order(&reopened, "demo").await,
            vec![json!("r1"), json!("r2"), json!("r3"), json!("waiting")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The last compaction of a dropped store outlives the drop, and a reopen
    /// reading the directory under it could fail or miss a segment.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn reopen_right_after_a_drop_waits_for_the_last_compaction() {
        for iteration in 0..200 {
            let dir = temp_store_dir();
            let store = FileStore::open(&dir, 1).await.unwrap();
            for n in 0..10 {
                store.enqueue("demo", json!(n)).await.unwrap();
            }
            if iteration % 2 == 0 {
                sleep(Duration::from_millis(1)).await;
            } else {
                tokio::task::yield_now().await;
            }
            for n in 10..20 {
                store.enqueue("demo", json!(n)).await.unwrap();
            }
            drop(store);

            let reopened = FileStore::open(&dir, HUGE_INTERVAL_MS).await.unwrap();
            assert_eq!(reopened.topic_stats("demo").await.depth, 20);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn open_starts_a_compactor_that_honours_the_interval() {
        let dir = temp_store_dir();
        let store = FileStore::open(&dir, 1).await.unwrap();
        store.enqueue("demo", json!("job")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            while snapshot_on_disk(&dir).revision == 0 {
                sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the compactor must fold the journal into the snapshot");
        let _ = std::fs::remove_dir_all(dir);
    }
}
