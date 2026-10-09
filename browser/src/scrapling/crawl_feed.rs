//! Where a crawl's items go besides the RPC sample: a worker-owned
//! `browser::crawl-item` trigger type for live observers, and a bounded
//! store read back through `browser::crawl::items` for recovery.
//!
//! This replaces the old optional `stream::set` feed (one stream group per
//! crawl, write failures ignored). Live delivery and the retained records are
//! both owned here, so the crawl works the same on an engine that runs no
//! stream worker.
//!
//! Bounds, all enforced here: at most [`MAX_BINDINGS`] bindings; one drain
//! task per crawl reading a queue of [`QUEUE_CAPACITY`] events (a full queue
//! drops the event and counts it, the crawl never waits on a consumer); event
//! items cut to [`EVENT_ITEM_BUDGET`]; and the store's crawl/byte/age limits.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::scrapling::page::budget_page_derived_fields;

/// The trigger type live observers bind.
pub const CRAWL_ITEM: &str = "browser::crawl-item";
/// The paginated read that recovers a crawl's retained items.
pub const ITEMS_FUNCTION: &str = "browser::crawl::items";

/// Bindings accepted across all crawls; registration fails above this.
pub const MAX_BINDINGS: usize = 64;
/// Events buffered per crawl for live delivery. A slow consumer fills it;
/// further events are dropped (and counted) instead of growing the queue.
pub const QUEUE_CAPACITY: usize = 256;
/// Page-derived fields of an event's `item` are cut to this many bytes.
pub const EVENT_ITEM_BUDGET: usize = 65_536;
/// Per-binding delivery timeout.
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a finished crawl waits for its queue to drain before returning.
pub const FLUSH_DEADLINE: Duration = Duration::from_secs(10);
/// Longest accepted crawl id.
pub const MAX_CRAWL_ID_BYTES: usize = 256;
/// `browser::crawl::items` page size: default and ceiling.
pub const DEFAULT_PAGE_LIMIT: usize = 20;
pub const MAX_PAGE_LIMIT: usize = 100;
/// Serialized item bytes per `browser::crawl::items` page (one item always
/// fits, however large).
pub const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;

/// Validate a caller-chosen crawl id (the `crawl_id` input and the binding
/// filter).
pub fn validate_crawl_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("`crawl_id` must not be empty".into());
    }
    if id.len() > MAX_CRAWL_ID_BYTES {
        return Err(format!(
            "`crawl_id` is longer than {MAX_CRAWL_ID_BYTES} bytes"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Trigger type: bindings
// ---------------------------------------------------------------------------

/// Config of a `browser::crawl-item` binding. Without `crawl_id` the binding
/// receives every crawl's items. Unknown keys are rejected so a misspelled
/// filter fails at registration instead of silently matching everything.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlItemBindingConfig {
    /// Only deliver events of this crawl (the `crawl_id` passed to, or
    /// returned as `crawl.id` by, `browser::crawl`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl_id: Option<String>,
}

/// What a `browser::crawl-item` binding receives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CrawlItemEvent {
    pub crawl_id: String,
    /// `item` for one crawled page, `done` once when the crawl ended.
    pub event: String,
    /// 1-based position in the crawl. Items carry 1..=n in completion order;
    /// `done` carries n + 1. A gap means a dropped event: read the missing
    /// positions with `browser::crawl::items`.
    pub seq: u64,
    /// `item` only: the crawl item, page-derived fields cut to 64 KiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Value>,
    /// `item` only: true when `item` was cut; the full one is retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// `done` only: the crawl's final `stats`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<Value>,
    /// `done` only: items retained for `browser::crawl::items`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained: Option<usize>,
    /// `done` only: item events dropped because this crawl's queue was full.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_events: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FeedBinding {
    pub id: String,
    pub function_id: String,
    pub crawl_id: Option<String>,
    pub namespace: Option<String>,
    pub metadata: Option<Value>,
}

/// The live `browser::crawl-item` bindings.
#[derive(Clone, Default)]
pub struct FeedBindings {
    inner: Arc<Mutex<HashMap<String, FeedBinding>>>,
}

impl FeedBindings {
    /// Validate and insert a binding (re-registering an id replaces it).
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let raw = if config.config.is_null() {
            json!({})
        } else {
            config.config.clone()
        };
        let filter: CrawlItemBindingConfig =
            serde_json::from_value(raw).map_err(|e| format!("invalid {CRAWL_ITEM} config: {e}"))?;
        if let Some(id) = &filter.crawl_id {
            validate_crawl_id(id).map_err(|e| format!("invalid {CRAWL_ITEM} config: {e}"))?;
        }
        let mut bindings = self.lock();
        if !bindings.contains_key(&config.id) && bindings.len() >= MAX_BINDINGS {
            return Err(format!(
                "too many {CRAWL_ITEM} bindings (max {MAX_BINDINGS}); unregister unused ones"
            ));
        }
        bindings.insert(
            config.id.clone(),
            FeedBinding {
                id: config.id,
                function_id: config.function_id,
                crawl_id: filter.crawl_id,
                namespace: config.namespace,
                metadata: config.metadata,
            },
        );
        Ok(())
    }

    pub fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Snapshot of the bindings that want `crawl_id`, so the lock is never
    /// held across a delivery.
    pub fn matching(&self, crawl_id: &str) -> Vec<FeedBinding> {
        self.lock()
            .values()
            .filter(|b| b.crawl_id.as_deref().is_none_or(|want| want == crawl_id))
            .cloned()
            .collect()
    }

    fn any_matching(&self, crawl_id: &str) -> bool {
        self.lock()
            .values()
            .any(|b| b.crawl_id.as_deref().is_none_or(|want| want == crawl_id))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, FeedBinding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct CrawlItemTriggerHandler {
    bindings: FeedBindings,
}

#[async_trait]
impl TriggerHandler for CrawlItemTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.bindings.add(config).map_err(Error::Handler)?;
        tracing::info!(id = %id, function_id = %function_id, "crawl-item binding registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.bindings.remove(&config.id);
        tracing::info!(id = %config.id, "crawl-item binding unregistered");
        Ok(())
    }
}

/// Register `browser::crawl-item` with the engine.
pub fn register_trigger_type(iii: &Arc<IIIClient>, hub: &CrawlFeedHub) {
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CRAWL_ITEM,
            "One page of a browser::crawl was crawled (event: item, in seq order), or the crawl \
             ended (event: done). Filter with crawl_id; bind before calling browser::crawl. \
             Dropped or missed items are read back with browser::crawl::items.",
            CrawlItemTriggerHandler {
                bindings: hub.bindings.clone(),
            },
        )
        .trigger_request_format::<CrawlItemBindingConfig>()
        .call_request_format::<CrawlItemEvent>(),
    );
    tracing::info!(trigger_type = CRAWL_ITEM, "registered trigger type");
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

/// How one event reaches one binding. Production invokes over the bus;
/// tests record.
#[async_trait]
pub trait FeedDelivery: Send + Sync {
    async fn deliver(&self, binding: &FeedBinding, payload: Value) -> Result<(), String>;
}

/// Awaited invocation of the bound function, in the binding's namespace and
/// with its metadata. Awaiting (rather than fire-and-forget) is what keeps a
/// consumer's events in `seq` order and lets a slow consumer fill the bounded
/// queue instead of an unbounded one elsewhere.
pub struct IiiFeedDelivery {
    iii: Arc<IIIClient>,
}

impl IiiFeedDelivery {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl FeedDelivery for IiiFeedDelivery {
    async fn deliver(&self, binding: &FeedBinding, payload: Value) -> Result<(), String> {
        let mut request = TriggerRequestWithMetadata::from(TriggerRequest {
            function_id: binding.function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(DELIVERY_TIMEOUT.as_millis() as u64),
        });
        if let Some(namespace) = &binding.namespace {
            request = request.namespace(namespace.clone());
        }
        if let Some(metadata) = &binding.metadata {
            request = request.metadata(metadata.clone());
        }
        self.iii
            .trigger(request)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// One task per crawl: events in queue order, each to every matching binding
/// (concurrently across bindings, sequentially across events).
async fn drain(
    mut rx: mpsc::Receiver<Value>,
    crawl_id: String,
    bindings: FeedBindings,
    delivery: Arc<dyn FeedDelivery>,
) {
    while let Some(event) = rx.recv().await {
        let targets = bindings.matching(&crawl_id);
        let deliveries = targets.iter().map(|binding| {
            let delivery = delivery.clone();
            let event = event.clone();
            async move {
                match tokio::time::timeout(DELIVERY_TIMEOUT, delivery.deliver(binding, event)).await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::debug!(
                        binding = %binding.id,
                        function_id = %binding.function_id,
                        error = %e,
                        "crawl-item delivery failed"
                    ),
                    Err(_) => tracing::debug!(
                        binding = %binding.id,
                        function_id = %binding.function_id,
                        "crawl-item delivery timed out"
                    ),
                }
            }
        });
        futures::future::join_all(deliveries).await;
    }
}

/// The live event for one item, its page-derived fields cut to the budget.
pub fn item_event(crawl_id: &str, seq: u64, item: &Value) -> CrawlItemEvent {
    let mut bounded = item.clone();
    if let Some(map) = bounded.as_object_mut() {
        budget_page_derived_fields(map, EVENT_ITEM_BUDGET);
    }
    let truncated = &bounded != item;
    CrawlItemEvent {
        crawl_id: crawl_id.to_string(),
        event: "item".into(),
        seq,
        item: Some(bounded),
        truncated: Some(truncated),
        stats: None,
        retained: None,
        dropped_events: None,
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct StoreLimits {
    /// Crawls kept; the oldest finished one is evicted for a new crawl.
    pub max_crawls: usize,
    /// How long a finished crawl stays readable.
    pub retention: Duration,
    /// Serialized item bytes retained per crawl.
    pub max_crawl_bytes: usize,
    /// Serialized item bytes retained across crawls; the oldest finished
    /// crawls are evicted first.
    pub max_total_bytes: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_crawls: 32,
            retention: Duration::from_secs(3600),
            max_crawl_bytes: 32 * 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
        }
    }
}

struct StoredCrawl {
    /// Items 1..=items.len(), always a prefix of the crawl: once one item does
    /// not fit, none after it is kept.
    items: Vec<(Value, usize)>,
    bytes: usize,
    started: Instant,
    finished: Option<Instant>,
    truncated: bool,
    stats: Option<Value>,
}

#[derive(Default)]
struct StoreInner {
    crawls: HashMap<String, StoredCrawl>,
    total_bytes: usize,
}

impl StoreInner {
    fn remove(&mut self, id: &str) {
        if let Some(old) = self.crawls.remove(id) {
            self.total_bytes -= old.bytes;
        }
    }

    fn oldest_finished_except(&self, keep: &str) -> Option<String> {
        self.crawls
            .iter()
            .filter(|(id, c)| c.finished.is_some() && id.as_str() != keep)
            .min_by_key(|(_, c)| (c.finished, c.started))
            .map(|(id, _)| id.clone())
    }
}

/// Worker-owned, bounded, in-memory record of recent crawls' items.
pub struct CrawlStore {
    inner: Mutex<StoreInner>,
    limits: StoreLimits,
}

impl CrawlStore {
    pub fn new(limits: StoreLimits) -> Self {
        Self {
            inner: Mutex::new(StoreInner::default()),
            limits,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn prune(&self, inner: &mut StoreInner) {
        let retention = self.limits.retention;
        let expired: Vec<String> = inner
            .crawls
            .iter()
            .filter(|(_, c)| c.finished.is_some_and(|at| at.elapsed() >= retention))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            inner.remove(&id);
        }
    }

    /// Open a crawl. A running crawl with the same id is an error; a finished
    /// one is replaced.
    pub fn begin(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        self.prune(&mut inner);
        if let Some(existing) = inner.crawls.get(id) {
            if existing.finished.is_none() {
                return Err(format!(
                    "crawl_id `{id}` is already running; pick another crawl_id"
                ));
            }
            inner.remove(id);
        }
        while inner.crawls.len() >= self.limits.max_crawls {
            match inner.oldest_finished_except(id) {
                Some(old) => inner.remove(&old),
                // Every slot is a running crawl: admit this one anyway; the
                // byte caps still bound memory.
                None => break,
            }
        }
        inner.crawls.insert(
            id.to_string(),
            StoredCrawl {
                items: Vec::new(),
                bytes: 0,
                started: Instant::now(),
                finished: None,
                truncated: false,
                stats: None,
            },
        );
        Ok(())
    }

    /// Retain the next item. Returns false (and marks the crawl truncated)
    /// when it does not fit the per-crawl or total byte caps.
    pub fn push(&self, id: &str, item: &Value) -> bool {
        let size = serde_json::to_vec(item).map(|v| v.len()).unwrap_or(0);
        let mut inner = self.lock();
        let Some(crawl) = inner.crawls.get(id) else {
            return false;
        };
        if crawl.truncated || crawl.bytes + size > self.limits.max_crawl_bytes {
            if let Some(crawl) = inner.crawls.get_mut(id) {
                crawl.truncated = true;
            }
            return false;
        }
        while inner.total_bytes + size > self.limits.max_total_bytes {
            match inner.oldest_finished_except(id) {
                Some(old) => inner.remove(&old),
                None => break,
            }
        }
        let fits = inner.total_bytes + size <= self.limits.max_total_bytes;
        let Some(crawl) = inner.crawls.get_mut(id) else {
            return false;
        };
        if !fits {
            crawl.truncated = true;
            return false;
        }
        crawl.items.push((item.clone(), size));
        crawl.bytes += size;
        inner.total_bytes += size;
        true
    }

    /// Close a crawl; `stats` is `None` when it was abandoned mid-way.
    /// Returns how many items were retained.
    pub fn finish(&self, id: &str, stats: Option<Value>) -> usize {
        let mut inner = self.lock();
        match inner.crawls.get_mut(id) {
            Some(crawl) => {
                crawl.finished = Some(Instant::now());
                crawl.stats = stats;
                crawl.items.len()
            }
            None => 0,
        }
    }

    /// One page of retained items after `after` (a seq), oldest first.
    pub fn page(&self, id: &str, after: u64, limit: usize) -> Result<Value, String> {
        let mut inner = self.lock();
        self.prune(&mut inner);
        let crawl = inner.crawls.get(id).ok_or_else(|| {
            format!(
                "unknown or expired crawl_id `{id}`: crawls are retained in memory for {} s after \
                 they finish (at most {} crawls) and not across worker restarts",
                self.limits.retention.as_secs(),
                self.limits.max_crawls
            )
        })?;
        let start = usize::try_from(after).unwrap_or(usize::MAX);
        let mut items = Vec::new();
        let mut bytes = 0usize;
        let mut next = start;
        for (index, (item, size)) in crawl.items.iter().enumerate().skip(start) {
            if items.len() >= limit || (!items.is_empty() && bytes + size > MAX_PAGE_BYTES) {
                break;
            }
            bytes += size;
            items.push(json!({"seq": index + 1, "item": item}));
            next = index + 1;
        }
        let mut out = serde_json::Map::new();
        out.insert("crawl_id".into(), json!(id));
        out.insert("items".into(), Value::Array(items));
        if next < crawl.items.len() {
            out.insert("next_after".into(), json!(next));
        }
        out.insert("retained".into(), json!(crawl.items.len()));
        out.insert("running".into(), json!(crawl.finished.is_none()));
        out.insert("truncated".into(), json!(crawl.truncated));
        if let Some(stats) = &crawl.stats {
            out.insert("stats".into(), stats.clone());
        }
        Ok(Value::Object(out))
    }
}

// ---------------------------------------------------------------------------
// Hub + per-crawl feed
// ---------------------------------------------------------------------------

/// Shared by every crawl: the bindings, the store, and how to deliver.
pub struct CrawlFeedHub {
    bindings: FeedBindings,
    store: Arc<CrawlStore>,
    delivery: Arc<dyn FeedDelivery>,
    queue_capacity: usize,
}

impl CrawlFeedHub {
    pub fn new(delivery: Arc<dyn FeedDelivery>) -> Self {
        Self::with_limits(delivery, StoreLimits::default(), QUEUE_CAPACITY)
    }

    pub fn with_limits(
        delivery: Arc<dyn FeedDelivery>,
        limits: StoreLimits,
        queue_capacity: usize,
    ) -> Self {
        Self {
            bindings: FeedBindings::default(),
            store: Arc::new(CrawlStore::new(limits)),
            delivery,
            queue_capacity: queue_capacity.max(1),
        }
    }

    pub fn bindings(&self) -> &FeedBindings {
        &self.bindings
    }

    /// Open the feed of one crawl. Must run inside a tokio runtime (it spawns
    /// the crawl's drain task).
    pub fn start(&self, crawl_id: &str) -> Result<CrawlFeed, String> {
        self.store.begin(crawl_id)?;
        let (tx, rx) = mpsc::channel(self.queue_capacity);
        let drain = tokio::spawn(drain(
            rx,
            crawl_id.to_string(),
            self.bindings.clone(),
            self.delivery.clone(),
        ));
        Ok(CrawlFeed {
            crawl_id: crawl_id.to_string(),
            store: self.store.clone(),
            bindings: self.bindings.clone(),
            tx: Some(tx),
            drain: Some(drain),
            seq: 0,
            dropped_events: 0,
            finished: false,
        })
    }

    /// `browser::crawl::items`.
    pub fn items(&self, payload: &Value) -> Result<Value, String> {
        let crawl_id = payload
            .get("crawl_id")
            .and_then(Value::as_str)
            .ok_or("provide `crawl_id` (the `crawl.id` browser::crawl returned)")?;
        let after = match payload.get("after") {
            None | Some(Value::Null) => 0,
            Some(value) => value
                .as_u64()
                .ok_or("`after` must be a non-negative integer (a seq)")?,
        };
        let limit = match payload.get("limit") {
            None | Some(Value::Null) => DEFAULT_PAGE_LIMIT,
            Some(value) => {
                let limit = value
                    .as_u64()
                    .filter(|limit| *limit >= 1)
                    .ok_or("`limit` must be an integer from 1 to 100")?;
                (limit as usize).min(MAX_PAGE_LIMIT)
            }
        };
        self.store.page(crawl_id, after, limit)
    }
}

/// What a finished crawl reports about its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedSummary {
    pub retained: usize,
    pub dropped_events: usize,
}

/// The feed of one running crawl.
pub struct CrawlFeed {
    crawl_id: String,
    store: Arc<CrawlStore>,
    bindings: FeedBindings,
    tx: Option<mpsc::Sender<Value>>,
    drain: Option<tokio::task::JoinHandle<()>>,
    seq: u64,
    dropped_events: usize,
    finished: bool,
}

impl CrawlFeed {
    pub fn crawl_id(&self) -> &str {
        &self.crawl_id
    }

    /// Record the next item and queue its live event. Never waits on a
    /// consumer: a full queue drops the event and counts it.
    pub fn push(&mut self, item: &Value) {
        self.seq += 1;
        self.store.push(&self.crawl_id, item);
        if !self.bindings.any_matching(&self.crawl_id) {
            return;
        }
        let event = match serde_json::to_value(item_event(&self.crawl_id, self.seq, item)) {
            Ok(event) => event,
            Err(e) => {
                tracing::warn!(error = %e, "crawl-item event failed to serialize");
                self.dropped_events += 1;
                return;
            }
        };
        let queued = self.tx.as_ref().map(|tx| tx.try_send(event).is_ok());
        if queued != Some(true) {
            self.dropped_events += 1;
        }
    }

    /// Close the crawl: retain its stats, queue the `done` event, and wait up
    /// to [`FLUSH_DEADLINE`] for live delivery to catch up. What is still
    /// queued after that keeps draining in the background.
    pub async fn finish(mut self, stats: &Value) -> FeedSummary {
        self.finished = true;
        let retained = self.store.finish(&self.crawl_id, Some(stats.clone()));
        let summary = FeedSummary {
            retained,
            dropped_events: self.dropped_events,
        };
        let deadline = tokio::time::Instant::now() + FLUSH_DEADLINE;
        if let Some(tx) = self.tx.take() {
            if self.bindings.any_matching(&self.crawl_id) {
                let done = CrawlItemEvent {
                    crawl_id: self.crawl_id.clone(),
                    event: "done".into(),
                    seq: self.seq + 1,
                    item: None,
                    truncated: None,
                    stats: Some(stats.clone()),
                    retained: Some(retained),
                    dropped_events: Some(self.dropped_events),
                };
                if let Ok(done) = serde_json::to_value(done) {
                    if tokio::time::timeout_at(deadline, tx.send(done))
                        .await
                        .is_err()
                    {
                        tracing::debug!(crawl_id = %self.crawl_id, "crawl-item done event not queued in time");
                    }
                }
            }
        }
        if let Some(drain) = self.drain.take() {
            if tokio::time::timeout_at(deadline, drain).await.is_err() {
                tracing::debug!(crawl_id = %self.crawl_id, "crawl-item delivery continues in background");
            }
        }
        summary
    }
}

impl Drop for CrawlFeed {
    /// A crawl whose call was cancelled still releases its id and becomes
    /// readable (without stats) instead of staying "running" forever.
    fn drop(&mut self) {
        if !self.finished {
            self.store.finish(&self.crawl_id, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Deprecated input
// ---------------------------------------------------------------------------

/// The response warning for a caller that still passes `stream_name`.
pub const STREAM_NAME_WARNING: &str = "stream_name is deprecated and ignored: browser::crawl no \
     longer writes items to a stream. Read every item with browser::crawl::items using crawl.id, \
     or bind the browser::crawl-item trigger (filter crawl_id) before calling for live items.";

static STREAM_NAME_LOGGED: AtomicBool = AtomicBool::new(false);

/// Warnings for deprecated inputs in a crawl request. Logs the first one
/// per process; every response carries them.
pub fn deprecated_input_warnings(payload: &Value, truthy: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut warnings = Vec::new();
    if payload.get("stream_name").is_some_and(truthy) {
        if !STREAM_NAME_LOGGED.swap(true, Ordering::Relaxed) {
            tracing::warn!("browser::crawl called with deprecated `stream_name`; it is ignored");
        }
        warnings.push(STREAM_NAME_WARNING.to_string());
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;

    fn binding(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: function_id.into(),
            config,
            metadata: Some(json!({"tenant": "t1"})),
            namespace: Some("ns-a".into()),
        }
    }

    /// Records (function_id, payload, namespace, metadata); optionally
    /// blocks every delivery until released, to play a slow consumer.
    /// (function_id, payload, namespace, metadata) per delivery.
    type Delivered = (String, Value, Option<String>, Option<Value>);

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<Delivered>>,
        gate: Option<Arc<Notify>>,
    }

    #[async_trait]
    impl FeedDelivery for Recorder {
        async fn deliver(&self, binding: &FeedBinding, payload: Value) -> Result<(), String> {
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            self.seen.lock().unwrap().push((
                binding.function_id.clone(),
                payload,
                binding.namespace.clone(),
                binding.metadata.clone(),
            ));
            Ok(())
        }
    }

    fn page_item(n: usize) -> Value {
        json!({"url": format!("https://e.com/{n}"), "status": 200, "content": format!("page {n}")})
    }

    #[test]
    fn binding_config_is_validated() {
        let set = FeedBindings::default();
        let err = set
            .add(binding("b", "f", json!({"crawl": "x"})))
            .unwrap_err();
        assert!(err.contains(CRAWL_ITEM), "{err}");
        let err = set
            .add(binding("b", "f", json!({"crawl_id": ""})))
            .unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
        let err = set
            .add(binding("b", "f", json!({"crawl_id": 3})))
            .unwrap_err();
        assert!(err.contains(CRAWL_ITEM), "{err}");
        set.add(binding("b", "f", Value::Null)).unwrap();
        set.add(binding("c", "f", json!({"crawl_id": "c1"})))
            .unwrap();
        assert_eq!(set.len(), 2);
        set.remove("b");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn bindings_are_capped() {
        let set = FeedBindings::default();
        for i in 0..MAX_BINDINGS {
            set.add(binding(&format!("b{i}"), "f", json!({}))).unwrap();
        }
        let err = set.add(binding("one-more", "f", json!({}))).unwrap_err();
        assert!(err.contains("too many"), "{err}");
        // Re-registering an existing id is a replacement, not a new binding.
        set.add(binding("b0", "g", json!({}))).unwrap();
        assert_eq!(set.len(), MAX_BINDINGS);
    }

    #[tokio::test]
    async fn items_reach_the_bound_consumer_in_order_filtered_by_crawl_id() {
        let recorder = Arc::new(Recorder::default());
        let hub = CrawlFeedHub::new(recorder.clone());
        hub.bindings()
            .add(binding("mine", "consumer::mine", json!({"crawl_id": "c1"})))
            .unwrap();
        hub.bindings()
            .add(binding(
                "other",
                "consumer::other",
                json!({"crawl_id": "c2"}),
            ))
            .unwrap();

        let mut feed = hub.start("c1").unwrap();
        for n in 1..=50 {
            feed.push(&page_item(n));
        }
        let stats = json!({"crawled": 50, "items": 50, "errors": 0, "stopped": "done"});
        let summary = feed.finish(&stats).await;
        assert_eq!(
            summary,
            FeedSummary {
                retained: 50,
                dropped_events: 0
            }
        );

        let seen = recorder.seen.lock().unwrap().clone();
        assert_eq!(
            seen.len(),
            51,
            "50 items + done, nothing for consumer::other"
        );
        for (i, (function_id, payload, namespace, metadata)) in seen.iter().enumerate() {
            assert_eq!(function_id, "consumer::mine");
            assert_eq!(namespace.as_deref(), Some("ns-a"), "binding namespace kept");
            assert_eq!(
                metadata,
                &Some(json!({"tenant": "t1"})),
                "binding metadata kept"
            );
            assert_eq!(payload["crawl_id"], "c1");
            assert_eq!(payload["seq"], json!(i + 1), "delivered in seq order");
            if i < 50 {
                assert_eq!(payload["event"], "item");
                assert_eq!(payload["item"], page_item(i + 1));
                assert_eq!(payload["truncated"], false);
            } else {
                assert_eq!(payload["event"], "done");
                assert_eq!(payload["stats"], stats);
                assert_eq!(payload["retained"], 50);
                assert_eq!(payload["dropped_events"], 0);
            }
        }
    }

    #[tokio::test]
    async fn unfiltered_binding_sees_every_crawl() {
        let recorder = Arc::new(Recorder::default());
        let hub = CrawlFeedHub::new(recorder.clone());
        hub.bindings()
            .add(binding("all", "consumer::all", json!({})))
            .unwrap();
        for id in ["a", "b"] {
            let mut feed = hub.start(id).unwrap();
            feed.push(&page_item(1));
            feed.finish(&json!({})).await;
        }
        let crawls: Vec<String> = recorder
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p, _, _)| {
                format!(
                    "{}:{}",
                    p["crawl_id"].as_str().unwrap(),
                    p["event"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(crawls, ["a:item", "a:done", "b:item", "b:done"]);
    }

    #[tokio::test]
    async fn no_binding_means_nothing_is_queued_but_everything_is_retained() {
        let recorder = Arc::new(Recorder::default());
        let hub = CrawlFeedHub::new(recorder.clone());
        hub.bindings()
            .add(binding(
                "other",
                "consumer::other",
                json!({"crawl_id": "zzz"}),
            ))
            .unwrap();
        let mut feed = hub.start("c1").unwrap();
        for n in 1..=5 {
            feed.push(&page_item(n));
        }
        let summary = feed.finish(&json!({"crawled": 5})).await;
        assert_eq!(summary.retained, 5);
        assert_eq!(summary.dropped_events, 0);
        assert!(recorder.seen.lock().unwrap().is_empty());
        let page = hub.items(&json!({"crawl_id": "c1"})).unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 5);
        assert_eq!(page["stats"], json!({"crawled": 5}));
    }

    #[tokio::test]
    async fn slow_consumer_fills_a_bounded_queue_and_recovers_from_the_store() {
        let gate = Arc::new(Notify::new());
        let recorder = Arc::new(Recorder {
            seen: Mutex::default(),
            gate: Some(gate.clone()),
        });
        let hub = CrawlFeedHub::with_limits(recorder.clone(), StoreLimits::default(), 4);
        hub.bindings()
            .add(binding("slow", "consumer::slow", json!({"crawl_id": "c1"})))
            .unwrap();

        let mut feed = hub.start("c1").unwrap();
        // Let the drain task pick up the first event and block on the gate.
        feed.push(&page_item(1));
        tokio::task::yield_now().await;
        for _ in 0..20 {
            if feed.tx.as_ref().unwrap().capacity() == 4 {
                break;
            }
            tokio::task::yield_now().await;
        }
        // The crawl keeps going: 4 more fit the queue, the rest are dropped
        // without blocking the producer.
        for n in 2..=30 {
            feed.push(&page_item(n));
        }
        assert_eq!(
            feed.dropped_events, 25,
            "queue holds 4 behind the in-flight one"
        );

        // Release the consumer while finish() waits for the drain.
        let releaser = tokio::spawn({
            let gate = gate.clone();
            async move {
                for _ in 0..64 {
                    gate.notify_one();
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        });
        let summary = feed.finish(&json!({"crawled": 30})).await;
        releaser.abort();
        assert_eq!(summary.retained, 30, "the store kept every item");
        assert_eq!(summary.dropped_events, 25);

        let seen = recorder.seen.lock().unwrap().clone();
        let seqs: Vec<u64> = seen
            .iter()
            .map(|(_, p, _, _)| p["seq"].as_u64().unwrap())
            .collect();
        assert_eq!(
            seqs,
            [1, 2, 3, 4, 5, 31],
            "still in order; done reports the gap"
        );
        assert_eq!(seen.last().unwrap().1["dropped_events"], 25);

        // Recovery: the consumer reads the missing seqs back.
        let page = hub
            .items(&json!({"crawl_id": "c1", "after": 5, "limit": 100}))
            .unwrap();
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), 25);
        assert_eq!(items[0]["seq"], 6);
        assert_eq!(items[0]["item"], page_item(6));
        assert_eq!(items[24]["seq"], 30);
        assert!(page.get("next_after").is_none());
        assert_eq!(page["running"], false);
    }

    #[test]
    fn event_items_are_bounded() {
        let big = json!({"url": "u", "content": "x".repeat(EVENT_ITEM_BUDGET * 2)});
        let event = item_event("c", 1, &big);
        assert_eq!(event.truncated, Some(true));
        let content = event.item.unwrap()["content"].as_str().unwrap().len();
        assert!(content <= EVENT_ITEM_BUDGET, "{content}");
        let small = item_event("c", 2, &page_item(2));
        assert_eq!(small.truncated, Some(false));
    }

    #[tokio::test]
    async fn running_crawl_id_is_rejected_and_finished_one_is_replaced() {
        let hub = CrawlFeedHub::new(Arc::new(Recorder::default()));
        let mut feed = hub.start("same").unwrap();
        let err = hub.start("same").err().unwrap();
        assert!(err.contains("already running"), "{err}");
        feed.push(&page_item(1));
        feed.finish(&json!({})).await;
        let mut again = hub.start("same").unwrap();
        again.push(&page_item(9));
        again.finish(&json!({})).await;
        let page = hub.items(&json!({"crawl_id": "same"})).unwrap();
        assert_eq!(page["items"], json!([{"seq": 1, "item": page_item(9)}]));
    }

    #[tokio::test]
    async fn a_dropped_feed_releases_its_id() {
        let hub = CrawlFeedHub::new(Arc::new(Recorder::default()));
        let mut feed = hub.start("cancelled").unwrap();
        feed.push(&page_item(1));
        drop(feed);
        let page = hub.items(&json!({"crawl_id": "cancelled"})).unwrap();
        assert_eq!(page["running"], false);
        assert!(page.get("stats").is_none());
        assert!(hub.start("cancelled").is_ok());
    }

    #[test]
    fn store_pages_and_validates_input() {
        let store = CrawlStore::new(StoreLimits::default());
        store.begin("c").unwrap();
        for n in 1..=45 {
            assert!(store.push("c", &page_item(n)));
        }
        let first = store.page("c", 0, DEFAULT_PAGE_LIMIT).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 20);
        assert_eq!(first["next_after"], 20);
        assert_eq!(first["running"], true);
        let last = store.page("c", 40, 20).unwrap();
        assert_eq!(last["items"].as_array().unwrap().len(), 5);
        assert!(last.get("next_after").is_none());
        assert!(store
            .page("nope", 0, 20)
            .unwrap_err()
            .contains("unknown or expired"));

        let hub = CrawlFeedHub::new(Arc::new(Recorder::default()));
        assert!(hub.items(&json!({})).unwrap_err().contains("crawl_id"));
        assert!(hub
            .items(&json!({"crawl_id": "x", "limit": 0}))
            .unwrap_err()
            .contains("limit"));
        assert!(hub
            .items(&json!({"crawl_id": "x", "after": -1}))
            .unwrap_err()
            .contains("after"));
    }

    #[test]
    fn store_caps_bytes_crawls_and_age() {
        let item = page_item(1);
        let size = serde_json::to_vec(&item).unwrap().len();
        let store = CrawlStore::new(StoreLimits {
            max_crawls: 2,
            retention: Duration::from_secs(3600),
            max_crawl_bytes: size * 3,
            max_total_bytes: size * 5,
        });
        store.begin("a").unwrap();
        for _ in 0..5 {
            store.push("a", &item);
        }
        assert_eq!(
            store.finish("a", Some(json!({}))),
            3,
            "per-crawl cap keeps a prefix"
        );
        assert_eq!(store.page("a", 0, 100).unwrap()["truncated"], true);

        store.begin("b").unwrap();
        for _ in 0..3 {
            assert!(store.push("b", &item));
        }
        // Total cap (5 items) is exceeded: the oldest finished crawl goes.
        assert!(store.page("a", 0, 1).is_err(), "a evicted for bytes");
        store.finish("b", Some(json!({})));

        store.begin("c").unwrap();
        store.finish("c", None);
        store.begin("d").unwrap();
        assert!(
            store.page("b", 0, 1).is_err(),
            "b evicted by the crawl-count cap"
        );
        assert!(store.page("c", 0, 1).is_ok());

        let expiring = CrawlStore::new(StoreLimits {
            retention: Duration::ZERO,
            ..StoreLimits::default()
        });
        expiring.begin("x").unwrap();
        expiring.finish("x", Some(json!({})));
        assert!(expiring.page("x", 0, 1).is_err(), "expired after retention");
    }

    #[test]
    fn stream_name_warns_and_is_otherwise_ignored() {
        let truthy = |v: &Value| !matches!(v, Value::Null | Value::Bool(false)) && v != "";
        assert!(deprecated_input_warnings(&json!({"url": "u"}), truthy).is_empty());
        assert!(deprecated_input_warnings(&json!({"stream_name": ""}), truthy).is_empty());
        let warnings = deprecated_input_warnings(&json!({"stream_name": "mine"}), truthy);
        assert_eq!(warnings, [STREAM_NAME_WARNING]);
        assert!(STREAM_NAME_WARNING.contains(ITEMS_FUNCTION));
        assert!(STREAM_NAME_WARNING.contains(CRAWL_ITEM));
    }
}
