//! Builtin transport adapter — subscribe model with per-subscriber
//! concurrency, fifo mode, and condition-function short-circuit, built on
//! top of the worker's own [`QueueStore`].
//!
//! Ported from the engine builtin `iii-queue` worker:
//! - `engine/src/workers/queue/adapters/builtin/adapter.rs` (the
//!   `QueueAdapter` impl, `FunctionHandler`, `SubscriptionConfig` mapping).
//! - `engine/src/builtins/queue.rs` (`QueueConfig` defaults, the
//!   `Worker`/`FifoWorker`/`GroupedFifoWorker` polling loops).
//!
//! Fan-out model (matches the engine exactly — see
//! `engine/src/workers/queue/adapters/builtin/adapter.rs:196-337`):
//! - Each `(topic, id)` subscription gets its own internal queue named
//!   `format!("{topic}::{function_id}")` (`adapter.rs:206,229`); `topic_functions`
//!   tracks which function ids are currently subscribed to a topic.
//! - `enqueue` pushes a **separate copy** of the message onto every
//!   subscribed function's internal queue when the topic has subscribers
//!   (`adapter.rs:203-215`, broadcast — N subscribers each receive every
//!   message, they do not compete for one shared queue); with no
//!   subscribers it enqueues straight onto the bare topic name
//!   (`adapter.rs:216-218`), where it sits until a subscriber later
//!   attaches (same buffering behavior as the engine).
//! - `redrive_dlq`, `redrive_dlq_message`, `discard_dlq_message`,
//!   `dlq_count`/`topic_stats`, and `dlq_peek` all resolve a bare topic
//!   name the same way: if it has subscribers, operate on/aggregate over
//!   every subscriber's internal queue (`adapter.rs:310-384,586-637`);
//!   otherwise operate on the bare topic directly. This lets service
//!   functions and operators keep passing the bare topic name for these
//!   ops regardless of how many subscribers exist.
//! - `list_topics` mirrors `adapter.rs:558-584`: topics are enumerated
//!   from the subscription registry (`topic_functions`), not by scanning
//!   every key the store happens to hold — a topic that only ever had a
//!   bare, subscriber-less `enqueue` does not appear (same as the engine).
//!
//! Deliberate deviations from the engine port:
//! - No `GroupedFifoWorker` (fifo + concurrency > 1, partitioned by
//!   `job.group_id`). This store's `Job` carries no group id, so fifo mode
//!   always processes strictly one job at a time regardless of the
//!   configured concurrency.
//! - Retries reuse [`QueueStore::nack`] (which already implements the
//!   engine's exponential backoff and DLQ-at-exhaustion) instead of the
//!   engine's `FifoWorker`, which retries a failed job in-place, blocking
//!   its single worker slot until it succeeds or is exhausted. Using
//!   `nack` for both modes means a retried fifo job goes back into the
//!   shared queue with a delay and may be interleaved with newer arrivals,
//!   rather than blocking the whole subscription on one job. Fifo
//!   *ordering* for jobs that succeed on their first attempt is unaffected
//!   (the poller is single-threaded and never dequeues the next job before
//!   the current one is acked or nacked).
//! - Duplicate `(topic, id)` subscriptions warn and no-op instead of
//!   silently replacing the old subscription's map entry and leaking its
//!   polling task (the actual, un-guarded, engine builtin behavior).
//!   Redis and RabbitMQ adapters in the engine already guard this way;
//!   this port applies the same guard to the builtin adapter to preserve
//!   the "one subscription per (topic, id)" invariant.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinHandle;

use crate::adapter::{QueueAdapter, TopicInfo};
use crate::store::{Job, QueueStore, TopicStats};
use crate::subscriber_config::SubscriberQueueConfig;
use crate::trigger::Invoker;

/// `engine/src/builtins/queue.rs` `QueueConfig::default()` (max_attempts).
const DEFAULT_MAX_RETRIES: u32 = 3;
/// `engine/src/builtins/queue.rs` `QueueConfig::default()` (backoff_ms).
const DEFAULT_BACKOFF_MS: u64 = 1000;
/// `engine/src/builtins/queue.rs` `QueueConfig::default()` (concurrency).
const DEFAULT_CONCURRENCY: u32 = 10;
/// `engine/src/builtins/queue.rs` `QueueConfig::default()` (poll_interval_ms).
const DEFAULT_POLL_INTERVAL_MS: u64 = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Concurrent,
    Fifo,
}

/// Per-subscription settings resolved from [`SubscriberQueueConfig`],
/// shared (read-only) across a subscription's polling task(s).
struct PollerConfig {
    /// The internal queue this subscription's poller reads from —
    /// `format!("{topic}::{function_id}")`, not the bare topic name (see
    /// the module doc's fan-out model).
    queue_name: String,
    function_id: String,
    condition_function_id: Option<String>,
    max_retries: u32,
    backoff_ms: u64,
    poll_interval_ms: u64,
}

/// A tracked `(topic, id)` subscription: its polling task plus the
/// function id it targets, so `unsubscribe` can decide whether any other
/// subscription still needs that function's entry in `topic_functions`
/// (mirrors the engine's `trigger_function_map`, `adapter.rs:50,273-308` —
/// folded into `subscriptions` here since the two maps always share the
/// same `(topic, id)` key set).
struct Subscription {
    task: JoinHandle<()>,
    function_id: String,
}

/// Builtin transport adapter: subscriptions are backed by pure in-process
/// polling tasks over the worker's [`QueueStore`].
pub struct BuiltinAdapter {
    store: Arc<dyn QueueStore>,
    invoker: Arc<dyn Invoker>,
    subscriptions: Mutex<HashMap<(String, String), Subscription>>,
    /// topic -> set of function ids currently subscribed to it. Resolves a
    /// bare topic name to its subscribers' internal queue names
    /// (`format!("{topic}::{function_id}")`) for fan-out enqueue and all
    /// DLQ/stat ops — mirrors the engine's `topic_functions`
    /// (`adapter.rs:49`).
    topic_functions: Mutex<HashMap<String, HashSet<String>>>,
    poll_interval_ms: u64,
}

/// `format!("{topic}::{function_id}")` — the internal queue name a
/// subscription's messages, acks/nacks, and DLQ entries live under.
/// Mirrors the engine's internal-queue naming (`adapter.rs:206,229`).
fn internal_queue_name(topic: &str, function_id: &str) -> String {
    format!("{topic}::{function_id}")
}

impl BuiltinAdapter {
    pub fn new(store: Arc<dyn QueueStore>, invoker: Arc<dyn Invoker>) -> Self {
        Self::with_poll_interval_ms(store, invoker, DEFAULT_POLL_INTERVAL_MS)
    }

    /// Test-only knob: a short poll interval keeps polling-loop-driven
    /// tests fast without changing observable subscribe/fifo/concurrency
    /// behavior.
    fn with_poll_interval_ms(
        store: Arc<dyn QueueStore>,
        invoker: Arc<dyn Invoker>,
        poll_interval_ms: u64,
    ) -> Self {
        Self {
            store,
            invoker,
            subscriptions: Mutex::new(HashMap::new()),
            topic_functions: Mutex::new(HashMap::new()),
            poll_interval_ms,
        }
    }
}

#[async_trait]
impl QueueAdapter for BuiltinAdapter {
    async fn enqueue(
        &self,
        topic: &str,
        data: Value,
        _traceparent: Option<String>,
        _baggage: Option<String>,
    ) {
        // This store's `Job` carries no trace context yet, so traceparent/
        // baggage are accepted (for trait parity) but dropped.
        //
        // Fan-out mirrors `adapter.rs:196-219`: a topic with subscribers
        // gets a separate copy pushed onto EACH subscriber's own internal
        // queue (broadcast); a topic with none enqueues straight onto the
        // bare topic name.
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    let _ = self.store.enqueue(&queue_name, data.clone()).await;
                }
            }
            _ => {
                let _ = self.store.enqueue(topic, data).await;
            }
        }
    }

    async fn subscribe(
        &self,
        topic: &str,
        id: &str,
        function_id: &str,
        condition_function_id: Option<String>,
        queue_config: Option<SubscriberQueueConfig>,
    ) {
        let key = (topic.to_string(), id.to_string());
        let mut subs = self.subscriptions.lock().await;
        if subs.contains_key(&key) {
            tracing::warn!(topic = %topic, id = %id, "Already subscribed to topic");
            return;
        }

        // No clamp: the engine has none either — `concurrency: 0` means
        // consumption is paused (the semaphore never yields a permit).
        let concurrency = queue_config
            .as_ref()
            .and_then(|c| c.concurrency)
            .unwrap_or(DEFAULT_CONCURRENCY);
        let max_retries = queue_config
            .as_ref()
            .and_then(|c| c.max_retries)
            .unwrap_or(DEFAULT_MAX_RETRIES);
        let backoff_ms = queue_config
            .as_ref()
            .and_then(|c| c.backoff_delay_ms)
            .unwrap_or(DEFAULT_BACKOFF_MS);
        let mode = match queue_config.as_ref().and_then(|c| c.queue_mode.as_deref()) {
            Some("fifo") => Mode::Fifo,
            _ => Mode::Concurrent,
        };

        let queue_name = internal_queue_name(topic, function_id);

        self.topic_functions
            .lock()
            .await
            .entry(topic.to_string())
            .or_default()
            .insert(function_id.to_string());

        let cfg = PollerConfig {
            queue_name,
            function_id: function_id.to_string(),
            condition_function_id,
            max_retries,
            backoff_ms,
            poll_interval_ms: self.poll_interval_ms,
        };

        let task = spawn_poller(
            Arc::clone(&self.store),
            Arc::clone(&self.invoker),
            cfg,
            mode,
            concurrency,
        );
        subs.insert(
            key,
            Subscription {
                task,
                function_id: function_id.to_string(),
            },
        );

        tracing::debug!(topic = %topic, id = %id, function_id = %function_id, ?mode, "Subscribed to queue via BuiltinAdapter");
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        let key = (topic.to_string(), id.to_string());
        let removed = self.subscriptions.lock().await.remove(&key);

        let Some(sub) = removed else {
            tracing::warn!(topic = %topic, id = %id, "No subscription found to unsubscribe");
            return;
        };
        sub.task.abort();
        tracing::debug!(topic = %topic, id = %id, "Unsubscribed from queue");

        // Only drop `function_id` from `topic_functions[topic]` once no
        // other subscription on this topic still targets it — mirrors the
        // engine's ref-counted cleanup (`adapter.rs:291-307`).
        let still_targeted = self
            .subscriptions
            .lock()
            .await
            .iter()
            .any(|((t, _), s)| t == topic && s.function_id == sub.function_id);

        if !still_targeted {
            let mut tf = self.topic_functions.lock().await;
            if let Some(function_ids) = tf.get_mut(topic) {
                function_ids.remove(&sub.function_id);
                if function_ids.is_empty() {
                    tf.remove(topic);
                }
            }
        }
    }

    async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64> {
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                let mut total = 0u64;
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    total += self.store.redrive_dlq(&queue_name).await;
                }
                Ok(total)
            }
            _ => Ok(self.store.redrive_dlq(topic).await),
        }
    }

    async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    if self
                        .store
                        .redrive_dlq_message(&queue_name, message_id)
                        .await
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(self.store.redrive_dlq_message(topic, message_id).await),
        }
    }

    async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    if self
                        .store
                        .discard_dlq_message(&queue_name, message_id)
                        .await
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(self.store.discard_dlq_message(topic, message_id).await),
        }
    }

    async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64> {
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                let mut total = 0u64;
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    total += self.store.topic_stats(&queue_name).await.dlq_depth;
                }
                Ok(total)
            }
            _ => Ok(self.store.topic_stats(topic).await.dlq_depth),
        }
    }

    async fn dlq_peek(&self, topic: &str, offset: u64, limit: u64) -> anyhow::Result<Vec<Value>> {
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        let jobs: Vec<Job> = match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                let mut all = Vec::new();
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    all.extend(
                        self.store
                            .dlq_messages(&queue_name, offset.saturating_add(limit))
                            .await,
                    );
                }
                all
            }
            _ => {
                self.store
                    .dlq_messages(topic, offset.saturating_add(limit))
                    .await
            }
        };
        Ok(jobs
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|job| serde_json::to_value(job).unwrap_or(Value::Null))
            .collect())
    }

    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        // Enumerated from the subscription registry, not by scanning every
        // key the store happens to hold — mirrors `adapter.rs:558-584`.
        let topic_functions = self.topic_functions.lock().await.clone();
        let mut infos = Vec::with_capacity(topic_functions.len());
        for (topic, function_ids) in &topic_functions {
            let mut depth = 0u64;
            for function_id in function_ids {
                let queue_name = internal_queue_name(topic, function_id);
                depth += self.store.topic_stats(&queue_name).await.depth;
            }
            infos.push(TopicInfo {
                name: topic.clone(),
                depth,
            });
        }
        Ok(infos)
    }

    async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats> {
        let function_ids = self.topic_functions.lock().await.get(topic).cloned();
        match function_ids {
            Some(function_ids) if !function_ids.is_empty() => {
                let mut aggregate = TopicStats::default();
                for function_id in &function_ids {
                    let queue_name = internal_queue_name(topic, function_id);
                    let stats = self.store.topic_stats(&queue_name).await;
                    aggregate.depth += stats.depth;
                    aggregate.dlq_depth += stats.dlq_depth;
                    aggregate.delivered += stats.delivered;
                    aggregate.failed += stats.failed;
                }
                Ok(aggregate)
            }
            _ => Ok(self.store.topic_stats(topic).await),
        }
    }

    async fn shutdown(&self) {
        let subs = self.subscriptions.lock().await.drain().collect::<Vec<_>>();
        for (_, sub) in subs {
            sub.task.abort();
        }
    }
}

fn spawn_poller(
    store: Arc<dyn QueueStore>,
    invoker: Arc<dyn Invoker>,
    cfg: PollerConfig,
    mode: Mode,
    concurrency: u32,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        match mode {
            Mode::Fifo => run_fifo(store, invoker, cfg).await,
            Mode::Concurrent => run_concurrent(store, invoker, cfg, concurrency).await,
        }
    })
}

/// Strictly one in-flight invocation, processed in dequeue order. Mirrors
/// the engine's `FifoWorker` (`concurrency <= 1` case) without the inline
/// retry-in-place behavior — see the module doc's "Deliberate deviations".
async fn run_fifo(store: Arc<dyn QueueStore>, invoker: Arc<dyn Invoker>, cfg: PollerConfig) {
    loop {
        match store.dequeue(&cfg.queue_name).await {
            Some(job) => process_job(&store, &invoker, &cfg, job).await,
            None => tokio::time::sleep(Duration::from_millis(cfg.poll_interval_ms)).await,
        }
    }
}

/// Up to `concurrency` in-flight invocations. Mirrors the engine's
/// `Worker`: a semaphore permit is held for the lifetime of each handler
/// invocation, gating how many jobs are processed concurrently.
async fn run_concurrent(
    store: Arc<dyn QueueStore>,
    invoker: Arc<dyn Invoker>,
    cfg: PollerConfig,
    concurrency: u32,
) {
    let semaphore = Arc::new(Semaphore::new(concurrency as usize));
    let cfg = Arc::new(cfg);

    loop {
        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return, // semaphore closed; subscription is being torn down
        };

        match store.dequeue(&cfg.queue_name).await {
            Some(job) => {
                let store = Arc::clone(&store);
                let invoker = Arc::clone(&invoker);
                let cfg = Arc::clone(&cfg);
                tokio::spawn(async move {
                    process_job(&store, &invoker, &cfg, job).await;
                    drop(permit);
                });
            }
            None => {
                drop(permit);
                tokio::time::sleep(Duration::from_millis(cfg.poll_interval_ms)).await;
            }
        }
    }
}

async fn process_job(
    store: &Arc<dyn QueueStore>,
    invoker: &Arc<dyn Invoker>,
    cfg: &PollerConfig,
    job: Job,
) {
    let result = invoke(
        invoker.as_ref(),
        &cfg.function_id,
        cfg.condition_function_id.as_deref(),
        job.payload.clone(),
    )
    .await;

    match result {
        Ok(()) => store.ack(&cfg.queue_name, &job.id).await,
        Err(_err) => {
            store
                .nack(&cfg.queue_name, job, cfg.max_retries, cfg.backoff_ms)
                .await
        }
    }
}

/// Port of the engine's `FunctionHandler::handle` condition-skip semantics
/// (`queue/src/trigger.rs:201-213`): only an explicit `Ok(Some(false))`
/// from the condition function skips (and acks, since it returns `Ok(())`
/// here); any other `Ok` continues to the target function; `Err` fails the
/// job (nacks).
async fn invoke(
    invoker: &dyn Invoker,
    function_id: &str,
    condition_function_id: Option<&str>,
    payload: Value,
) -> Result<(), String> {
    if let Some(condition_id) = condition_function_id {
        match invoker.call(condition_id, payload.clone()).await {
            Ok(Some(Value::Bool(false))) => return Ok(()),
            Ok(_) => {}
            Err(err) => return Err(err),
        }
    }
    invoker.call(function_id, payload).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryStore;
    use serde_json::json;
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::time::{sleep, Instant};

    async fn wait_until<F, Fut>(mut pred: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if pred().await {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(pred().await, "condition did not become true before timeout");
    }

    /// Cheap deterministic jitter so ordering tests exercise varied handler
    /// latency without adding a `rand` dependency.
    fn jitter_ms(seed: u64) -> u64 {
        let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % 15) + 1
    }

    #[derive(Default)]
    struct FakeInvoker {
        calls: Mutex<Vec<(String, Value)>>,
        fail_backend: AtomicBool,
        condition_value: Mutex<Option<Value>>,
    }

    impl FakeInvoker {
        async fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().await.clone()
        }
    }

    #[async_trait]
    impl Invoker for FakeInvoker {
        async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
            self.calls
                .lock()
                .await
                .push((function_id.to_string(), payload));
            if function_id == "condition" {
                return Ok(self.condition_value.lock().await.clone());
            }
            if self.fail_backend.load(Ordering::SeqCst) {
                Err("backend failed".to_string())
            } else {
                Ok(Some(json!({"ok": true})))
            }
        }
    }

    /// Tracks the maximum number of concurrent in-flight `call`s observed.
    #[derive(Default)]
    struct ConcurrencyGateInvoker {
        in_flight: AtomicUsize,
        max_seen: AtomicUsize,
        hold_ms: u64,
    }

    impl ConcurrencyGateInvoker {
        fn new(hold_ms: u64) -> Self {
            Self {
                in_flight: AtomicUsize::new(0),
                max_seen: AtomicUsize::new(0),
                hold_ms,
            }
        }

        fn max_seen(&self) -> usize {
            self.max_seen.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Invoker for ConcurrencyGateInvoker {
        async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(now, Ordering::SeqCst);
            sleep(Duration::from_millis(self.hold_ms)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(Some(json!({"ok": true})))
        }
    }

    /// Records the order (and payload) of every `call`, sleeping a jittered
    /// delay first so a broken fifo poller (e.g. one that spawns handlers
    /// concurrently) would visibly scramble the order.
    #[derive(Default)]
    struct OrderRecordingInvoker {
        order: Mutex<Vec<i64>>,
    }

    impl OrderRecordingInvoker {
        async fn order(&self) -> Vec<i64> {
            self.order.lock().await.clone()
        }
    }

    #[async_trait]
    impl Invoker for OrderRecordingInvoker {
        async fn call(&self, _function_id: &str, payload: Value) -> Result<Option<Value>, String> {
            let n = payload.as_i64().unwrap_or_default();
            sleep(Duration::from_millis(jitter_ms(n as u64))).await;
            self.order.lock().await.push(n);
            Ok(Some(json!({"ok": true})))
        }
    }

    fn config(overrides: SubscriberQueueConfig) -> Option<SubscriberQueueConfig> {
        Some(overrides)
    }

    // (a) subscribe+enqueue delivers.
    #[tokio::test]
    async fn subscribe_then_enqueue_delivers() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 5);

        adapter
            .subscribe("demo", "sub1", "backend", None, None)
            .await;
        adapter
            .enqueue("demo", json!({"hello": "world"}), None, None)
            .await;

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.calls().await.len() == 1 }
        })
        .await;
        assert_eq!(adapter.topic_stats("demo").await.unwrap().delivered, 1);
        adapter.shutdown().await;
    }

    // (g) enqueue fans out a separate copy to EVERY subscriber on the same
    // topic (broadcast) instead of the subscribers competing for one
    // shared queue — mirrors `adapter.rs:203-215`.
    #[tokio::test]
    async fn enqueue_fans_out_to_every_subscriber() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 5);

        adapter.subscribe("demo", "sub-a", "fn-a", None, None).await;
        adapter.subscribe("demo", "sub-b", "fn-b", None, None).await;

        for i in 0..3 {
            adapter.enqueue("demo", json!(i), None, None).await;
        }

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.calls().await.len() == 6 }
        })
        .await;

        let calls = invoker.calls().await;
        let fn_a_calls = calls.iter().filter(|(f, _)| f == "fn-a").count();
        let fn_b_calls = calls.iter().filter(|(f, _)| f == "fn-b").count();
        assert_eq!(fn_a_calls, 3, "fn-a should receive every enqueued message");
        assert_eq!(fn_b_calls, 3, "fn-b should receive every enqueued message");
        adapter.shutdown().await;
    }

    // enqueue onto a topic with no subscribers buffers directly on the
    // bare topic name (no internal-queue namespacing) — mirrors
    // `adapter.rs:216-218`.
    #[tokio::test]
    async fn enqueue_without_subscribers_buffers_on_bare_topic() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker, 5);

        adapter
            .enqueue("demo", json!("no subscribers yet"), None, None)
            .await;

        assert_eq!(store.topic_stats("demo").await.depth, 1);
        let job = store
            .dequeue("demo")
            .await
            .expect("job should be on the bare topic");
        assert_eq!(job.payload, json!("no subscribers yet"));
    }

    // concurrency: 0 pauses consumption entirely — no clamp, matching the
    // engine (a semaphore with zero permits never yields one).
    #[tokio::test]
    async fn concurrency_zero_pauses_consumption() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 5);

        adapter
            .subscribe(
                "demo",
                "sub1",
                "backend",
                None,
                config(SubscriberQueueConfig {
                    concurrency: Some(0),
                    ..Default::default()
                }),
            )
            .await;
        adapter
            .enqueue("demo", json!({"hello": "world"}), None, None)
            .await;

        sleep(Duration::from_millis(200)).await;
        assert!(
            invoker.calls().await.is_empty(),
            "concurrency: 0 must not deliver any message"
        );
        adapter.shutdown().await;
    }

    // (b) concurrency=3 -> 3 in-flight invocations observed, never more.
    #[tokio::test]
    async fn concurrency_limits_in_flight_invocations() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(ConcurrencyGateInvoker::new(60));
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 5);

        adapter
            .subscribe(
                "demo",
                "sub1",
                "backend",
                None,
                config(SubscriberQueueConfig {
                    concurrency: Some(3),
                    ..Default::default()
                }),
            )
            .await;
        for i in 0..12 {
            adapter.enqueue("demo", json!(i), None, None).await;
        }

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.max_seen() >= 3 }
        })
        .await;
        // Give any (incorrect) over-eager spawning a chance to show up.
        sleep(Duration::from_millis(100)).await;
        assert_eq!(invoker.max_seen(), 3);
        adapter.shutdown().await;
    }

    // (c) fifo -> strict ordering under 20 enqueues with random handler latency.
    #[tokio::test]
    async fn fifo_processes_strictly_in_order() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(OrderRecordingInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 3);

        adapter
            .subscribe(
                "demo",
                "sub1",
                "backend",
                None,
                config(SubscriberQueueConfig {
                    queue_mode: Some("fifo".to_string()),
                    // Deliberately set >1: fifo mode must ignore it (no
                    // grouped-fifo support) and still process one at a time.
                    concurrency: Some(5),
                    ..Default::default()
                }),
            )
            .await;
        for i in 0..20 {
            adapter.enqueue("demo", json!(i), None, None).await;
        }

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.order().await.len() == 20 }
        })
        .await;
        let order = invoker.order().await;
        let expected: Vec<i64> = (0..20).collect();
        assert_eq!(order, expected);
        adapter.shutdown().await;
    }

    // (d) failing handler -> DLQ after max_retries. Also exercises finding
    // 1's requirement that DLQ ops keep working against the bare topic
    // name a service function passes, even though the job actually lives
    // under the internal `demo::backend` queue.
    #[tokio::test]
    async fn failing_handler_moves_to_dlq_after_max_retries() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        invoker.fail_backend.store(true, Ordering::SeqCst);
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 3);

        adapter
            .subscribe(
                "demo",
                "sub1",
                "backend",
                None,
                config(SubscriberQueueConfig {
                    max_retries: Some(2),
                    backoff_delay_ms: Some(1),
                    ..Default::default()
                }),
            )
            .await;
        adapter.enqueue("demo", json!("job"), None, None).await;

        wait_until(|| async { adapter.dlq_count("demo").await.unwrap_or(0) > 0 }).await;

        assert_eq!(adapter.dlq_count("demo").await.unwrap(), 1);
        let peeked = adapter.dlq_peek("demo", 0, 10).await.unwrap();
        assert_eq!(peeked.len(), 1);
        assert_eq!(peeked[0]["attempts"], json!(2));
        adapter.shutdown().await;
    }

    // DLQ ops (redrive_dlq, discard_dlq_message, dlq_count, dlq_peek)
    // resolve a bare topic across MULTIPLE subscribers by aggregating over
    // each one's internal queue — mirrors `adapter.rs:310-384`.
    #[tokio::test]
    async fn dlq_ops_resolve_bare_topic_across_multiple_subscribers() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        invoker.fail_backend.store(true, Ordering::SeqCst);
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 3);

        let retry_cfg = || {
            config(SubscriberQueueConfig {
                max_retries: Some(1),
                backoff_delay_ms: Some(1),
                ..Default::default()
            })
        };
        adapter
            .subscribe("demo", "sub-a", "fn-a", None, retry_cfg())
            .await;
        adapter
            .subscribe("demo", "sub-b", "fn-b", None, retry_cfg())
            .await;

        adapter.enqueue("demo", json!("job"), None, None).await;

        // Both subscribers get their own copy and both fail straight to
        // their own DLQ (max_retries: 1) -> dlq_count aggregates both.
        wait_until(|| async { adapter.dlq_count("demo").await.unwrap_or(0) >= 2 }).await;
        assert_eq!(adapter.dlq_count("demo").await.unwrap(), 2);

        let peeked = adapter.dlq_peek("demo", 0, 10).await.unwrap();
        assert_eq!(peeked.len(), 2);

        let job_id = peeked[0]["id"].as_str().unwrap().to_string();
        assert!(adapter.discard_dlq_message("demo", &job_id).await.unwrap());
        assert_eq!(adapter.dlq_count("demo").await.unwrap(), 1);

        let redriven = adapter.redrive_dlq("demo").await.unwrap();
        assert_eq!(redriven, 1);
        assert_eq!(adapter.dlq_count("demo").await.unwrap(), 0);

        adapter.shutdown().await;
    }

    // (e) unsubscribe stops delivery.
    #[tokio::test]
    async fn unsubscribe_stops_delivery() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 3);

        adapter
            .subscribe("demo", "sub1", "backend", None, None)
            .await;
        adapter.unsubscribe("demo", "sub1").await;
        store.enqueue("demo", json!("after")).await.unwrap();
        sleep(Duration::from_millis(120)).await;
        assert!(invoker.calls().await.is_empty());
    }

    // (f) condition=false skips + acks without invoking the target function.
    #[tokio::test]
    async fn condition_false_skips_and_acks_without_invoking_target() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        *invoker.condition_value.lock().await = Some(Value::Bool(false));
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 3);

        adapter
            .subscribe(
                "demo",
                "sub1",
                "backend",
                Some("condition".to_string()),
                None,
            )
            .await;
        adapter
            .enqueue("demo", json!({"hello": "world"}), None, None)
            .await;

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.calls().await.len() == 1 }
        })
        .await;
        assert_eq!(invoker.calls().await[0].0, "condition");
        assert_eq!(adapter.topic_stats("demo").await.unwrap().delivered, 1);
        adapter.shutdown().await;
    }

    // Duplicate (topic, id) subscribe is a warn+no-op, not a silent replace.
    #[tokio::test]
    async fn duplicate_subscription_is_noop() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker.clone(), 3);

        adapter
            .subscribe("demo", "sub1", "backend", None, None)
            .await;
        adapter
            .subscribe("demo", "sub1", "other-backend", None, None)
            .await;
        assert_eq!(adapter.subscriptions.lock().await.len(), 1);
        adapter.shutdown().await;
    }

    // dlq_peek returns store DLQ jobs as JSON.
    #[tokio::test]
    async fn dlq_peek_returns_store_jobs_as_json() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let adapter = BuiltinAdapter::with_poll_interval_ms(store.clone(), invoker, 3);

        store.enqueue("demo", json!("job")).await.unwrap();
        let job = store.dequeue("demo").await.unwrap();
        store.nack("demo", job, 1, 1).await;

        let peeked = adapter.dlq_peek("demo", 0, 10).await.unwrap();
        assert_eq!(peeked.len(), 1);
        assert_eq!(peeked[0]["payload"], json!("job"));
        assert_eq!(peeked[0]["attempts"], json!(1));
    }
}
