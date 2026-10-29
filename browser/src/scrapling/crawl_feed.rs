//! Where a crawl's items go besides the RPC sample: the worker's private
//! `browser::on-crawl-item` function, called once per event so its observers
//! receive the crawl live, and a bounded store read back through
//! `browser::crawl::items` for recovery.
//!
//! Live delivery and the retained records are both owned here, so the crawl
//! works the same on an engine that runs no stream worker.
//!
//! `browser::on-crawl-item` delivers its calls itself: each event goes to the
//! observers of its crawl (args `{ crawl_id }`) and to the observers of every
//! crawl (no args), and the call returns once each of their targets answered
//! or timed out. One drain task per crawl makes the calls in `seq` order, so
//! a slow consumer slows its crawl's queue, never the crawl.
//!
//! Bounds, all enforced here: at most [`MAX_OBSERVERS`] observers; one drain
//! task per crawl reading a queue of [`QUEUE_CAPACITY`] events (a full queue
//! drops the event and counts it, the crawl never waits on a consumer); event
//! items cut to [`EVENT_ITEM_BUDGET`]; and the store's crawl/byte/age limits.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    Call, CallRequest, IIIClient, Keys, Match, ObserveRejection, Observer, RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::scrapling::page::budget_page_derived_fields;

/// The function live observers observe.
pub const ON_CRAWL_ITEM_FN: &str = "browser::on-crawl-item";
/// The paginated read that recovers a crawl's retained items.
pub const ITEMS_FUNCTION: &str = "browser::crawl::items";

/// Observers accepted across all crawls; `observe` fails above this.
pub const MAX_OBSERVERS: usize = 64;
/// Events buffered per crawl for live delivery. A slow consumer fills it;
/// further events are dropped (and counted) instead of growing the queue.
pub const QUEUE_CAPACITY: usize = 256;
/// Page-derived fields of an event's `item` are cut to this many bytes.
pub const EVENT_ITEM_BUDGET: usize = 65_536;
/// Per-observer delivery timeout.
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the drain waits for one call of `browser::on-crawl-item`: the
/// observers are called concurrently, each bounded by [`DELIVERY_TIMEOUT`].
const CALL_TIMEOUT: Duration = Duration::from_secs(6);
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

/// The key an observer of every crawl is filed under: every call lists it.
const ALL_CRAWLS: &str = "*";

/// Validate a caller-chosen crawl id (the `crawl_id` input and the observer
/// arg).
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
// The function: observers
// ---------------------------------------------------------------------------

/// Args of a `browser::on-crawl-item` observer. Without `crawl_id` the
/// observer receives every crawl's items. Unknown keys are refused
/// (`INVALID_ARGS`) so a misspelled key fails at `observe` instead of
/// silently matching everything.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrawlItemArgs {
    /// Only this crawl's events (the `crawl_id` passed to, or returned as
    /// `crawl.id` by, `browser::crawl`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl_id: Option<String>,
}

/// What `browser::on-crawl-item` returns, and what its observers' targets
/// receive.
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

/// The accepted observers of `browser::on-crawl-item`, by id: what the cap
/// counts. The engine holds everything else about them.
#[derive(Clone, Default)]
pub struct CrawlObservers {
    inner: Arc<Mutex<HashSet<String>>>,
}

impl CrawlObservers {
    /// Validate an observer's args and accept it (accepting an id again is
    /// not a new observer). `INVALID_ARGS` for a bad `crawl_id`,
    /// `OBSERVER_LIMIT` past [`MAX_OBSERVERS`].
    pub fn admit(&self, id: &str, args: &CrawlItemArgs) -> Result<(), ObserveRejection> {
        if let Some(crawl_id) = &args.crawl_id {
            validate_crawl_id(crawl_id).map_err(|e| {
                ObserveRejection::new("INVALID_ARGS", format!("{ON_CRAWL_ITEM_FN}: {e}"))
            })?;
        }
        let mut observers = self.lock();
        if !observers.contains(id) && observers.len() >= MAX_OBSERVERS {
            return Err(ObserveRejection::new(
                "OBSERVER_LIMIT",
                format!(
                    "too many {ON_CRAWL_ITEM_FN} observers (max {MAX_OBSERVERS}); dispose unused \
                     ones"
                ),
            ));
        }
        observers.insert(id.to_string());
        Ok(())
    }

    pub fn release(&self, id: &str) {
        self.lock().remove(id);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The key an observer with these args is filed under: its crawl, or
/// [`ALL_CRAWLS`].
fn observer_key(args: &Value) -> &str {
    args.get("crawl_id")
        .and_then(Value::as_str)
        .unwrap_or(ALL_CRAWLS)
}

/// The keys an event with this input matches: its crawl's and
/// [`ALL_CRAWLS`].
fn call_keys(input: &Value) -> Vec<&str> {
    match input.get("crawl_id").and_then(Value::as_str) {
        Some(crawl_id) => vec![crawl_id, ALL_CRAWLS],
        None => vec![ALL_CRAWLS],
    }
}

/// Calls each matching observer's target with the event, concurrently, each
/// bounded by [`DELIVERY_TIMEOUT`], and returns once they all answered or
/// timed out. The caller (the crawl's drain) gets `null`.
async fn deliver(iii: &IIIClient, call: Call, observers: Vec<Observer>) -> Result<Value, Error> {
    let event = call.output.clone().unwrap_or_else(|| call.input.clone());
    let deliveries = observers
        .iter()
        .filter(|obs| obs.matches(&call))
        .map(|obs| {
            let request = CallRequest::observer(obs, event.clone()).timeout(DELIVERY_TIMEOUT);
            async move {
                match tokio::time::timeout(DELIVERY_TIMEOUT, iii.call(request)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => tracing::debug!(
                        observer_id = obs.id(),
                        target = obs.target(),
                        error = %e,
                        "crawl-item delivery failed"
                    ),
                    Err(_) => tracing::debug!(
                        observer_id = obs.id(),
                        target = obs.target(),
                        "crawl-item delivery timed out"
                    ),
                }
            }
        });
    futures::future::join_all(deliveries).await;
    Ok(Value::Null)
}

/// Register `browser::on-crawl-item`: private, its handler returns the event
/// it is called with, args `{ crawl_id? }`. Its hooks are set by id first, so
/// they see every observer: the added hook validates `crawl_id` and caps the
/// observers, the match hook gives each event to the observers of its crawl
/// and of every crawl, and the deliver hook calls their targets.
pub fn register_function(iii: &Arc<IIIClient>, hub: &CrawlFeedHub) {
    let observers = hub.observers.clone();
    iii.on_observer_added_for(ON_CRAWL_ITEM_FN, move |obs: Observer| {
        let observers = observers.clone();
        async move {
            let args: CrawlItemArgs = obs.args()?;
            observers.admit(obs.id(), &args)?;
            tracing::info!(
                observer_id = obs.id(),
                target = obs.target(),
                "crawl-item observer added"
            );
            Ok(())
        }
    });
    let observers = hub.observers.clone();
    iii.on_observer_removed_for(ON_CRAWL_ITEM_FN, move |obs: Observer| {
        let observers = observers.clone();
        async move {
            observers.release(obs.id());
            tracing::info!(observer_id = obs.id(), "crawl-item observer removed");
        }
    });
    iii.on_observer_match_for(
        ON_CRAWL_ITEM_FN,
        Match::keys(
            |obs| Keys::from(observer_key(obs.args_value())),
            |call| Keys::from(call_keys(&call.input)),
        ),
    );
    let engine = iii.clone();
    iii.on_observer_deliver_for(
        ON_CRAWL_ITEM_FN,
        move |call: Call, observers: Vec<Observer>| {
            let engine = engine.clone();
            async move { deliver(&engine, call, observers).await }
        },
    );
    iii.register_function(
        ON_CRAWL_ITEM_FN,
        RegisterFunction::new(|event: CrawlItemEvent| Ok::<_, Error>(event))
            .args_format::<CrawlItemArgs>()
            .private()
            .description(
                "One page of a browser::crawl was crawled (event: item, in seq order), or the \
                 crawl ended (event: done). Returns the CrawlItemEvent it is called with. Args \
                 { crawl_id? }: observe with the crawl_id before calling browser::crawl; without \
                 it, every crawl. Each target receives the CrawlItemEvent, and a slow target \
                 slows its crawl's live feed. Dropped or missed items are read back with \
                 browser::crawl::items. Private: only the browser worker calls it.",
            ),
    );
    tracing::info!(function_id = ON_CRAWL_ITEM_FN, "registered event function");
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

/// Where a crawl's live events go. Production calls `browser::on-crawl-item`;
/// tests record.
#[async_trait]
pub trait FeedSink: Send + Sync {
    /// Whether an event of `crawl_id` would reach any observer.
    fn is_observed(&self, crawl_id: &str) -> bool;
    /// Deliver one event; returns once every matching observer's target
    /// answered (or timed out).
    async fn deliver(&self, event: Value) -> Result<(), String>;
}

/// Awaited calls of `browser::on-crawl-item`. Awaiting (rather than
/// fire-and-forget) is what keeps a consumer's events in `seq` order and lets
/// a slow consumer fill the bounded queue instead of an unbounded one
/// elsewhere.
pub struct IiiFeedSink {
    iii: Arc<IIIClient>,
}

impl IiiFeedSink {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl FeedSink for IiiFeedSink {
    fn is_observed(&self, crawl_id: &str) -> bool {
        let call = Call {
            input: json!({ "crawl_id": crawl_id }),
            ..Call::default()
        };
        self.iii.is_observed_for(ON_CRAWL_ITEM_FN, &call)
    }

    async fn deliver(&self, event: Value) -> Result<(), String> {
        self.iii
            .call(CallRequest {
                function_id: ON_CRAWL_ITEM_FN.to_string(),
                payload: event,
                action: None,
                timeout_ms: Some(CALL_TIMEOUT.as_millis() as u64),
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// One task per crawl: events in queue order, each delivered before the
/// next.
async fn drain(mut rx: mpsc::Receiver<Value>, crawl_id: String, sink: Arc<dyn FeedSink>) {
    while let Some(event) = rx.recv().await {
        if let Err(e) = sink.deliver(event).await {
            tracing::debug!(crawl_id = %crawl_id, error = %e, "crawl-item delivery failed");
        }
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

/// Shared by every crawl: the accepted observers, the store, and where live
/// events go.
pub struct CrawlFeedHub {
    observers: CrawlObservers,
    store: Arc<CrawlStore>,
    sink: Arc<dyn FeedSink>,
    queue_capacity: usize,
}

impl CrawlFeedHub {
    pub fn new(sink: Arc<dyn FeedSink>) -> Self {
        Self::with_limits(sink, StoreLimits::default(), QUEUE_CAPACITY)
    }

    pub fn with_limits(
        sink: Arc<dyn FeedSink>,
        limits: StoreLimits,
        queue_capacity: usize,
    ) -> Self {
        Self {
            observers: CrawlObservers::default(),
            store: Arc::new(CrawlStore::new(limits)),
            sink,
            queue_capacity: queue_capacity.max(1),
        }
    }

    /// The accepted `browser::on-crawl-item` observers.
    pub fn observers(&self) -> &CrawlObservers {
        &self.observers
    }

    /// Open the feed of one crawl. Must run inside a tokio runtime (it spawns
    /// the crawl's drain task).
    pub fn start(&self, crawl_id: &str) -> Result<CrawlFeed, String> {
        self.store.begin(crawl_id)?;
        let (tx, rx) = mpsc::channel(self.queue_capacity);
        let drain = tokio::spawn(drain(rx, crawl_id.to_string(), self.sink.clone()));
        Ok(CrawlFeed {
            crawl_id: crawl_id.to_string(),
            store: self.store.clone(),
            sink: self.sink.clone(),
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
    sink: Arc<dyn FeedSink>,
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
        if !self.sink.is_observed(&self.crawl_id) {
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
            if self.sink.is_observed(&self.crawl_id) {
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
pub const STREAM_NAME_WARNING: &str = "stream_name is deprecated and ignored: browser::crawl \
     writes items to no stream. Read every item with browser::crawl::items using crawl.id, or \
     observe browser::on-crawl-item (args { crawl_id }) before calling for live items.";

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

    /// Observes the crawls in `watched` (`*` for every crawl) and records
    /// every delivered event; optionally blocks every delivery until
    /// released, to play a slow consumer.
    #[derive(Default)]
    struct Recorder {
        watched: Vec<&'static str>,
        seen: Mutex<Vec<Value>>,
        gate: Option<Arc<Notify>>,
    }

    impl Recorder {
        fn watching(watched: &[&'static str]) -> Self {
            Self {
                watched: watched.to_vec(),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl FeedSink for Recorder {
        fn is_observed(&self, crawl_id: &str) -> bool {
            self.watched
                .iter()
                .any(|w| *w == ALL_CRAWLS || *w == crawl_id)
        }

        async fn deliver(&self, event: Value) -> Result<(), String> {
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            self.seen.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn page_item(n: usize) -> Value {
        json!({"url": format!("https://e.com/{n}"), "status": 200, "content": format!("page {n}")})
    }

    #[test]
    fn observer_args_are_validated() {
        for bad in [json!({"crawl": "x"}), json!({"crawl_id": 3})] {
            assert!(
                serde_json::from_value::<CrawlItemArgs>(bad.clone()).is_err(),
                "{bad} must be refused"
            );
        }
        let schema = serde_json::to_value(schemars::schema_for!(CrawlItemArgs)).unwrap();
        assert_eq!(schema["additionalProperties"], json!(false));

        let observers = CrawlObservers::default();
        let empty = CrawlItemArgs {
            crawl_id: Some(String::new()),
        };
        let err = observers.admit("o", &empty).unwrap_err();
        assert_eq!(err.code, "INVALID_ARGS");
        assert!(err.message.contains("must not be empty"), "{}", err.message);
        let long = CrawlItemArgs {
            crawl_id: Some("x".repeat(MAX_CRAWL_ID_BYTES + 1)),
        };
        assert_eq!(
            observers.admit("o", &long).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert!(observers.is_empty(), "a refused observer is not counted");

        observers.admit("a", &CrawlItemArgs::default()).unwrap();
        observers
            .admit(
                "b",
                &CrawlItemArgs {
                    crawl_id: Some("c1".into()),
                },
            )
            .unwrap();
        assert_eq!(observers.len(), 2);
        observers.release("a");
        assert_eq!(observers.len(), 1);
    }

    #[test]
    fn observers_are_capped() {
        let observers = CrawlObservers::default();
        for i in 0..MAX_OBSERVERS {
            observers
                .admit(&format!("o{i}"), &CrawlItemArgs::default())
                .unwrap();
        }
        let err = observers
            .admit("one-more", &CrawlItemArgs::default())
            .unwrap_err();
        assert_eq!(err.code, "OBSERVER_LIMIT");
        // Accepting an id again is not a new observer.
        observers.admit("o0", &CrawlItemArgs::default()).unwrap();
        assert_eq!(observers.len(), MAX_OBSERVERS);
        observers.release("o1");
        observers
            .admit("one-more", &CrawlItemArgs::default())
            .unwrap();
    }

    #[test]
    fn an_event_matches_the_observers_of_its_crawl_and_of_every_crawl() {
        assert_eq!(observer_key(&json!({"crawl_id": "c1"})), "c1");
        assert_eq!(observer_key(&json!({})), ALL_CRAWLS);
        assert_eq!(
            call_keys(&json!({"crawl_id": "c1", "seq": 1})),
            ["c1", ALL_CRAWLS]
        );
        assert_eq!(call_keys(&json!({})), [ALL_CRAWLS]);
    }

    #[tokio::test]
    async fn items_of_an_observed_crawl_are_delivered_in_order() {
        let recorder = Arc::new(Recorder::watching(&["c1"]));
        let hub = CrawlFeedHub::new(recorder.clone());

        let mut other = hub.start("c2").unwrap();
        other.push(&page_item(1));
        other.finish(&json!({})).await;
        assert!(
            recorder.seen.lock().unwrap().is_empty(),
            "nobody observes c2: nothing delivered"
        );

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
        assert_eq!(seen.len(), 51, "50 items + done");
        for (i, payload) in seen.iter().enumerate() {
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
    async fn an_observer_of_every_crawl_sees_every_crawl() {
        let recorder = Arc::new(Recorder::watching(&[ALL_CRAWLS]));
        let hub = CrawlFeedHub::new(recorder.clone());
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
            .map(|p| {
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
    async fn no_observer_means_nothing_is_queued_but_everything_is_retained() {
        let recorder = Arc::new(Recorder::watching(&["zzz"]));
        let hub = CrawlFeedHub::new(recorder.clone());
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
            watched: vec!["c1"],
            seen: Mutex::default(),
            gate: Some(gate.clone()),
        });
        let hub = CrawlFeedHub::with_limits(recorder.clone(), StoreLimits::default(), 4);

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
        let seqs: Vec<u64> = seen.iter().map(|p| p["seq"].as_u64().unwrap()).collect();
        assert_eq!(
            seqs,
            [1, 2, 3, 4, 5, 31],
            "still in order; done reports the gap"
        );
        assert_eq!(seen.last().unwrap()["dropped_events"], 25);

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
        assert!(STREAM_NAME_WARNING.contains(ON_CRAWL_ITEM_FN));
    }
}
