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
//! Deliberate deviations from the engine port:
//! - No per-function internal-queue fan-out (`topic::function_id`). The
//!   engine redirects each subscribed function to its own copy of every
//!   message; this port hands `enqueue` straight to the shared
//!   [`QueueStore`] topic, matching this task's brief ("enqueue delegates
//!   to store"). Two subscribers on the same topic therefore compete for
//!   messages rather than each receiving a copy.
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

use std::collections::HashMap;
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
    topic: String,
    function_id: String,
    condition_function_id: Option<String>,
    max_retries: u32,
    backoff_ms: u64,
    poll_interval_ms: u64,
}

/// Builtin transport adapter: subscriptions are backed by pure in-process
/// polling tasks over the worker's [`QueueStore`].
pub struct BuiltinAdapter {
    store: Arc<dyn QueueStore>,
    invoker: Arc<dyn Invoker>,
    subscriptions: Mutex<HashMap<(String, String), JoinHandle<()>>>,
    poll_interval_ms: u64,
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
        let _ = self.store.enqueue(topic, data).await;
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

        let concurrency = queue_config
            .as_ref()
            .and_then(|c| c.concurrency)
            .unwrap_or(DEFAULT_CONCURRENCY)
            .max(1);
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

        let cfg = PollerConfig {
            topic: topic.to_string(),
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
        subs.insert(key, task);

        tracing::debug!(topic = %topic, id = %id, function_id = %function_id, ?mode, "Subscribed to queue via BuiltinAdapter");
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        let key = (topic.to_string(), id.to_string());
        let mut subs = self.subscriptions.lock().await;
        if let Some(task) = subs.remove(&key) {
            task.abort();
            tracing::debug!(topic = %topic, id = %id, "Unsubscribed from queue");
        } else {
            tracing::warn!(topic = %topic, id = %id, "No subscription found to unsubscribe");
        }
    }

    async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64> {
        Ok(self.store.redrive_dlq(topic).await)
    }

    async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        Ok(self.store.redrive_dlq_message(topic, message_id).await)
    }

    async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        Ok(self.store.discard_dlq_message(topic, message_id).await)
    }

    async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64> {
        Ok(self.store.topic_stats(topic).await.dlq_depth)
    }

    async fn dlq_peek(&self, topic: &str, offset: u64, limit: u64) -> anyhow::Result<Vec<Value>> {
        let jobs = self
            .store
            .dlq_messages(topic, offset.saturating_add(limit))
            .await;
        Ok(jobs
            .into_iter()
            .skip(offset as usize)
            .map(|job| serde_json::to_value(job).unwrap_or(Value::Null))
            .collect())
    }

    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        let mut infos = Vec::new();
        for name in self.store.list_topics().await {
            let depth = self.store.topic_stats(&name).await.depth;
            infos.push(TopicInfo { name, depth });
        }
        Ok(infos)
    }

    async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats> {
        Ok(self.store.topic_stats(topic).await)
    }

    async fn shutdown(&self) {
        let tasks = self.subscriptions.lock().await.drain().collect::<Vec<_>>();
        for (_, task) in tasks {
            task.abort();
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
        match store.dequeue(&cfg.topic).await {
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

        match store.dequeue(&cfg.topic).await {
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
        Ok(()) => store.ack(&cfg.topic, &job.id).await,
        Err(_err) => {
            store
                .nack(&cfg.topic, job, cfg.max_retries, cfg.backoff_ms)
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
        assert_eq!(store.topic_stats("demo").await.delivered, 1);
        adapter.shutdown().await;
    }

    // (b) concurrency=3 -> 3 in-flight invocations observed, never more.
    #[tokio::test]
    async fn concurrency_limits_in_flight_invocations() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        for i in 0..12 {
            store.enqueue("demo", json!(i)).await.unwrap();
        }
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
        for i in 0..20 {
            store.enqueue("demo", json!(i)).await.unwrap();
        }
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

    // (d) failing handler -> DLQ after max_retries.
    #[tokio::test]
    async fn failing_handler_moves_to_dlq_after_max_retries() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        store.enqueue("demo", json!("job")).await.unwrap();
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

        wait_until(|| {
            let store = store.clone();
            async move { !store.dlq_messages("demo", 10).await.is_empty() }
        })
        .await;
        let stats = store.topic_stats("demo").await;
        assert_eq!(stats.failed, 1);
        assert_eq!(store.dlq_messages("demo", 10).await[0].attempts, 2);
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
        store
            .enqueue("demo", json!({"hello": "world"}))
            .await
            .unwrap();
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

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.calls().await.len() == 1 }
        })
        .await;
        assert_eq!(invoker.calls().await[0].0, "condition");
        assert_eq!(store.topic_stats("demo").await.delivered, 1);
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
