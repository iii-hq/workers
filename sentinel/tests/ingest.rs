//! The ingest pipeline end to end, over a real store (see support/db.rs).
//!
//! The engine is faked because its telemetry is what we are simulating; the
//! store is not, because what these tests are really checking is that a tick
//! becomes the right row.

#[path = "support/db.rs"]
mod db;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use db::TestDb;
use sentinel::ingest::{
    ring::PhantomRing, CheckoutVersions, Ingest, IngestJob, Telemetry, TICK_BACKLOG_MAX,
};
use sentinel::registry::{EngineRegistry, FunctionEntry, Registry, WorkerEntry};
use sentinel::store::Db;
use sentinel::{
    Counters, GroupStatusV1, SentinelError, Store, TraceStoreStateV1, TraceSummary, WorkerConfig,
};
use serde_json::{json, Value};

const NOW_NANOS: u64 = 1_789_000_000_500_000_000;
/// What the engine answers, as the SDK and the runtime render it, with no span store.
const SPAN_STORE_DISABLED: &str = "engine::traces::list: remote error \
     (memory_exporter_not_enabled): In-memory span storage is not available. \
     Set exporter: memory or both in config.";

#[derive(Default)]
struct FakeEngineTelemetry {
    summaries: BTreeMap<String, TraceSummary>,
    trees: BTreeMap<String, Vec<Value>>,
    logs: BTreeMap<String, Vec<Value>>,
    /// Every read of the summaries fails with this.
    read_error: Option<String>,
    /// The next this-many reads time out, then reads answer again.
    failures_left: Arc<AtomicU64>,
    /// A trace a read before this time does not see yet.
    visible_at: BTreeMap<String, tokio::time::Instant>,
    /// A read before this sees nothing: the engine has broadcast the spans and
    /// not stored them yet.
    stored_at: Option<tokio::time::Instant>,
    reads: Arc<AtomicU64>,
}

#[async_trait]
impl Telemetry for FakeEngineTelemetry {
    async fn traces(&self, trace_ids: &[String]) -> Result<Vec<TraceSummary>, SentinelError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if let Some(message) = &self.read_error {
            return Err(SentinelError::dependency(message));
        }
        if self
            .failures_left
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(SentinelError::dependency(
                "engine::traces::list: invocation timed out",
            ));
        }
        if self
            .stored_at
            .is_some_and(|at| tokio::time::Instant::now() < at)
        {
            return Ok(Vec::new());
        }
        let now = tokio::time::Instant::now();
        Ok(trace_ids
            .iter()
            .filter(|id| self.visible_at.get(*id).is_none_or(|at| now >= *at))
            .filter_map(|id| self.summaries.get(id).cloned())
            .collect())
    }

    async fn tree(&self, trace_id: &str) -> Result<Vec<Value>, SentinelError> {
        Ok(self.trees.get(trace_id).cloned().unwrap_or_default())
    }

    async fn logs(&self, trace_id: &str) -> Result<Vec<Value>, SentinelError> {
        Ok(self.logs.get(trace_id).cloned().unwrap_or_default())
    }
}

struct FakeRegistry;

#[async_trait]
impl EngineRegistry for FakeRegistry {
    async fn list_functions(&self) -> Result<Vec<FunctionEntry>, SentinelError> {
        Ok(vec![
            FunctionEntry {
                function_id: "state::compare-and-set".into(),
                namespace: "default".into(),
                worker_name: "state".into(),
            },
            FunctionEntry {
                function_id: "queue::deliver".into(),
                namespace: "default".into(),
                worker_name: "queue".into(),
            },
        ])
    }

    async fn list_workers(&self) -> Result<Vec<WorkerEntry>, SentinelError> {
        Ok(vec![
            WorkerEntry {
                name: "state".into(),
                namespace: "default".into(),
                version: Some("0.23.0".into()),
            },
            WorkerEntry {
                name: "queue".into(),
                namespace: "default".into(),
                version: Some("0.22.1-dev".into()),
            },
        ])
    }
}

struct FakeCheckouts;

#[async_trait]
impl CheckoutVersions for FakeCheckouts {
    async fn version_for(&self, worker: &str) -> Option<String> {
        // Only `queue` reports a development version here.
        (worker == "queue").then(|| "git:4662b0d".to_string())
    }
}

struct Harness {
    ingest: Ingest<TestDb, FakeRegistry>,
    store: Arc<Store<TestDb>>,
    counters: Arc<Counters>,
    ring: Arc<PhantomRing>,
    config: WorkerConfig,
}

async fn harness(telemetry: FakeEngineTelemetry) -> Harness {
    let store = Arc::new(Store::new(db::test_db().await));
    store.migrate().await.expect("migrate");
    let counters = Arc::new(Counters::default());
    let ring = Arc::new(PhantomRing::default());
    let ingest = Ingest::new(
        store.clone(),
        Arc::new(telemetry),
        Arc::new(Registry::new(FakeRegistry, Duration::from_secs(60))),
        Arc::new(FakeCheckouts),
        counters.clone(),
        ring.clone(),
    );
    Harness {
        ingest,
        store,
        counters,
        ring,
        config: WorkerConfig::default(),
    }
}

fn summary(trace_id: &str, service: &str, function: &str, tags: &[(&str, &str)]) -> TraceSummary {
    TraceSummary {
        trace_id: trace_id.into(),
        status: "error".into(),
        error_count: 1,
        service_name: Some(service.into()),
        function_id: Some(function.into()),
        name: Some(format!("execute {function}")),
        span_count: 0,
        trace_tags: tags
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    }
}

fn span(id: &str, name: &str, service: &str, status: &str, children: Value) -> Value {
    json!({
        "span_id": id,
        "name": name,
        "service_name": service,
        "status": status,
        "start_time_unix_nano": NOW_NANOS,
        "end_time_unix_nano": NOW_NANOS + 500_000,
        "attributes": [],
        "events": [],
        "links": [],
        "children": children,
    })
}

fn with_parent(mut value: Value, parent: &str) -> Value {
    value["parent_span_id"] = json!(parent);
    value
}

/// The failure this worker was built for: a compare-and-set rejected inside a
/// harness turn, with the callee's own span missing so the failing span is
/// the engine's `call` hop.
fn cas_trace() -> (TraceSummary, Vec<Value>) {
    let mut leaf = with_parent(
        span(
            "leaf",
            "call state::compare-and-set",
            "iii",
            "error",
            json!([]),
        ),
        "step",
    );
    leaf["status_description"] = json!("expected version 41, found 42");
    leaf["events"] = json!([{
        "name": "exception",
        "timestamp_unix_nano": NOW_NANOS,
        "attributes": [
            ["exception.type", "CasMismatch"],
            ["exception.message", "expected version 41, found 42"],
        ],
    }]);
    let step = with_parent(
        span(
            "step",
            "harness::turn step",
            "harness",
            "error",
            json!([leaf]),
        ),
        "root",
    );
    let root = span(
        "root",
        "execute harness::send",
        "harness",
        "error",
        json!([step]),
    );
    (
        summary(
            "t1",
            "harness",
            "harness::send",
            &[("iii.session.id", "s_7a1"), ("iii.message.id", "t_12")],
        ),
        vec![root],
    )
}

async fn count(store: &Store<TestDb>, sql: &str) -> i64 {
    store
        .db()
        .query(sql, vec![])
        .await
        .expect("query")
        .first()
        .and_then(|row| row.values().next().cloned())
        .and_then(|value| sentinel::store::integer(&value))
        .unwrap_or(0)
}

async fn one(store: &Store<TestDb>, sql: &str) -> Value {
    store
        .db()
        .query(sql, vec![])
        .await
        .expect("query")
        .first()
        .and_then(|row| row.values().next().cloned())
        .map(db::plain)
        .unwrap_or(Value::Null)
}

fn clean(trace_id: &str) -> TraceSummary {
    TraceSummary {
        trace_id: trace_id.into(),
        status: "ok".into(),
        ..TraceSummary::default()
    }
}

fn ids(trace_ids: &[&str]) -> Vec<String> {
    trace_ids.iter().map(|id| id.to_string()).collect()
}

#[tokio::test(start_paused = true)]
async fn a_tick_of_one_failing_and_two_clean_traces_queues_one_job_that_records_the_group() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    telemetry.summaries.insert("c1".into(), clean("c1"));
    telemetry.summaries.insert("c2".into(), clean("c2"));
    let reads = telemetry.reads.clone();
    let harness = harness(telemetry).await;

    let queued = harness
        .ingest
        .filter_tick(&ids(&["c1", "t1", "c2"]))
        .await
        .0;

    assert_eq!(queued, ids(&["t1"]), "a clean trace must cost no job");
    assert_eq!(reads.load(Ordering::Relaxed), 1, "the tick is one read");
    assert_eq!(harness.counters.trace_store(), TraceStoreStateV1::Memory);
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 0);

    let report = harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: queued[0].clone(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");
    assert_eq!(report.recorded, 1);
    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        1
    );
}

#[tokio::test(start_paused = true)]
async fn a_tick_keeps_a_trace_by_its_error_count_or_by_its_status_in_any_case() {
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert(
        "counted".into(),
        TraceSummary {
            error_count: 2,
            ..clean("counted")
        },
    );
    telemetry.summaries.insert(
        "tagged".into(),
        TraceSummary {
            status: "ERROR".into(),
            ..clean("tagged")
        },
    );
    let harness = harness(telemetry).await;

    assert_eq!(
        harness
            .ingest
            .filter_tick(&ids(&["counted", "tagged"]))
            .await
            .0,
        ids(&["counted", "tagged"])
    );
}

/// The engine broadcasts a span before it stores it, and the last span of a
/// trace never ticks again: a read on the tick would lose that error for good.
#[tokio::test(start_paused = true)]
async fn a_span_the_engine_stores_just_after_its_tick_is_still_seen() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    telemetry.stored_at = Some(tokio::time::Instant::now() + Duration::from_millis(100));
    let harness = harness(telemetry).await;

    assert_eq!(
        harness.ingest.filter_tick(&ids(&["t1"])).await.0,
        ids(&["t1"])
    );
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 0);
}

#[tokio::test(start_paused = true)]
async fn a_read_failure_queues_nothing_and_reads_the_ids_again_after_a_backoff() {
    let (summary, _) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.summaries.insert("c1".into(), clean("c1"));
    telemetry.summaries.insert("c2".into(), clean("c2"));
    telemetry.failures_left.store(1, Ordering::Relaxed);
    let reads = telemetry.reads.clone();
    let harness = harness(telemetry).await;

    assert!(
        harness
            .ingest
            .filter_tick(&ids(&["t1", "c1"]))
            .await
            .0
            .is_empty(),
        "a failed read must not turn into a job, and a read, per trace"
    );
    assert!(
        harness.ingest.filter_tick(&ids(&["c2"])).await.0.is_empty(),
        "inside the backoff a tick only adds its ids"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 1);

    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        harness.ingest.filter_tick(&[]).await.0,
        ids(&["t1"]),
        "one read takes every id that waited, so the error is not lost"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 2);
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 0);
}

#[tokio::test(start_paused = true)]
async fn ticks_that_arrive_during_a_read_join_it_instead_of_starting_their_own() {
    let (summary, _) = cas_trace();
    let mut other = summary.clone();
    other.trace_id = "t2".into();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.summaries.insert("t2".into(), other);
    let reads = telemetry.reads.clone();
    let harness = harness(telemetry).await;

    let (t1, t2) = (ids(&["t1"]), ids(&["t2"]));
    let (first, second) = tokio::join!(
        harness.ingest.filter_tick(&t1),
        harness.ingest.filter_tick(&t2),
    );

    assert_eq!(first.0, ids(&["t1", "t2"]));
    assert!(second.0.is_empty(), "its id rode on the read in flight");
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn a_backlog_past_its_cap_drops_the_oldest_ids_and_counts_them() {
    let telemetry = FakeEngineTelemetry {
        read_error: Some("engine::traces::list: invocation timed out".into()),
        ..FakeEngineTelemetry::default()
    };
    let harness = harness(telemetry).await;
    let many: Vec<String> = (0..TICK_BACKLOG_MAX + 10)
        .map(|n| format!("t{n}"))
        .collect();

    assert!(harness.ingest.filter_tick(&many).await.0.is_empty());
    let counts = harness.counters.ingest_snapshot(0);
    assert_eq!(counts.tick_backlog_dropped, 10);
    assert_eq!(
        counts.lost_before_capture, 0,
        "the engine's ring lost nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn a_trace_that_ticks_again_while_waiting_is_read_one_window_after_its_latest_tick() {
    let (summary, _) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    // A second tick 250 ms in carries t1's error span, stored 100 ms after it.
    telemetry.visible_at.insert(
        "t1".into(),
        tokio::time::Instant::now() + Duration::from_millis(350),
    );
    let harness = harness(telemetry).await;

    let t1 = ids(&["t1"]);
    let (first, second) = tokio::join!(harness.ingest.filter_tick(&t1), async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        harness.ingest.filter_tick(&t1).await
    });

    assert_eq!(first.0, ids(&["t1"]), "read after the second tick's window");
    assert!(second.0.is_empty());
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 0);
}

#[tokio::test(start_paused = true)]
async fn after_a_failed_read_the_backlog_says_when_to_read_it_again() {
    let (summary, _) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.failures_left.store(1, Ordering::Relaxed);
    let harness = harness(telemetry).await;
    assert!(harness.ingest.tick_backlog_due().is_none());

    let started = tokio::time::Instant::now();
    assert!(harness.ingest.filter_tick(&ids(&["t1"])).await.0.is_empty());
    let due = harness
        .ingest
        .tick_backlog_due()
        .expect("the failed id waits for a later read");
    assert!(
        due >= started + Duration::from_millis(1300),
        "not before the backoff that follows the read at 300 ms"
    );

    tokio::time::sleep_until(due).await;
    assert_eq!(
        harness.ingest.filter_tick(&[]).await.0,
        ids(&["t1"]),
        "a read with no new tick drains what waited"
    );
    assert!(harness.ingest.tick_backlog_due().is_none());

    assert!(harness.ingest.claim_tick_drain());
    assert!(
        !harness.ingest.claim_tick_drain(),
        "one scheduled read at a time"
    );
    harness.ingest.release_tick_drain();
    assert!(harness.ingest.claim_tick_drain());
}

#[tokio::test(start_paused = true)]
async fn an_id_that_joins_a_read_in_flight_still_waits_for_the_engine_to_store_its_span() {
    let (summary, _) = cas_trace();
    let mut other = summary.clone();
    other.trace_id = "t2".into();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.summaries.insert("t2".into(), other);
    // t2 ticks 250 ms into t1's read delay; the engine stores its span 100 ms later.
    telemetry.visible_at.insert(
        "t2".into(),
        tokio::time::Instant::now() + Duration::from_millis(350),
    );
    let harness = harness(telemetry).await;

    let (t1, t2) = (ids(&["t1"]), ids(&["t2"]));
    let (first, second) = tokio::join!(harness.ingest.filter_tick(&t1), async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        harness.ingest.filter_tick(&t2).await
    });

    assert_eq!(
        first.0,
        ids(&["t1", "t2"]),
        "t2 is read one window after its own tick, not at t1's"
    );
    assert!(second.0.is_empty());
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 0);
}

#[tokio::test(start_paused = true)]
async fn a_disabled_span_store_queues_nothing_and_says_so() {
    let telemetry = FakeEngineTelemetry {
        read_error: Some(SPAN_STORE_DISABLED.into()),
        ..FakeEngineTelemetry::default()
    };
    let harness = harness(telemetry).await;

    assert!(harness
        .ingest
        .filter_tick(&ids(&["t1", "t2"]))
        .await
        .0
        .is_empty());
    assert_eq!(harness.counters.trace_store(), TraceStoreStateV1::Disabled);
}

#[tokio::test(start_paused = true)]
async fn a_trace_gone_from_the_answer_counts_as_lost_at_the_tick() {
    let (summary, _) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    let harness = harness(telemetry).await;

    assert_eq!(
        harness.ingest.filter_tick(&ids(&["t1", "gone"])).await.0,
        ids(&["t1"])
    );
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 1);
}

#[tokio::test]
async fn a_log_runs_on_its_own_when_the_engine_has_no_span_store() {
    let telemetry = FakeEngineTelemetry {
        read_error: Some(SPAN_STORE_DISABLED.into()),
        ..FakeEngineTelemetry::default()
    };
    let harness = harness(telemetry).await;

    harness
        .ingest
        .handle(
            IngestJob::Log {
                trace_id: "t1".into(),
                log: json!({
                    "timestamp_unix_nano": NOW_NANOS,
                    "severity_text": "ERROR",
                    "body": "compare-and-set rejected",
                    "attributes": {},
                    "trace_id": "t1",
                    "span_id": "leaf",
                    "service_name": "harness",
                }),
            },
            &harness.config,
        )
        .await
        .expect("a missing span store is a state, not a failure for the queue to retry");

    assert_eq!(harness.counters.trace_store(), TraceStoreStateV1::Disabled);
    assert_eq!(
        count(
            &harness.store,
            "SELECT COUNT(*) FROM sentinel_occurrences WHERE pending_join = 1"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_tick_of_only_this_workers_own_traces_reads_nothing() {
    let telemetry = FakeEngineTelemetry::default();
    let reads = telemetry.reads.clone();
    let harness = harness(telemetry).await;
    harness.ring.remember("own");

    let (queued, phantom_dropped) = harness.ingest.filter_tick(&ids(&["own"])).await;

    assert!(queued.is_empty());
    assert_eq!(phantom_dropped, 1);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn a_tick_that_names_its_errored_traces_queues_them_with_no_read() {
    let telemetry = FakeEngineTelemetry::default();
    let reads = telemetry.reads.clone();
    let harness = harness(telemetry).await;
    harness.ring.remember("own");

    let (queued, phantom_dropped) = harness
        .ingest
        .filter_errored_tick(&ids(&["own", "t1"]))
        .await;

    assert_eq!(queued, ids(&["t1"]));
    assert_eq!(
        phantom_dropped, 1,
        "this worker's own trace is still dropped"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0, "no engine::traces::list");
    assert!(harness.ingest.filter_errored_tick(&[]).await.0.is_empty());
}

/// The engine broadcasts a span before it stores it. A named trace queued on
/// the tick itself would have its job read a trace with no error yet, record
/// nothing, and never be queued again.
#[tokio::test(start_paused = true)]
async fn a_named_trace_the_engine_stores_just_after_its_tick_is_still_recorded() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    telemetry.stored_at = Some(tokio::time::Instant::now() + Duration::from_millis(100));
    let harness = harness(telemetry).await;

    let (queued, _) = harness.ingest.filter_errored_tick(&ids(&["t1"])).await;
    let report = harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: queued[0].clone(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(report.recorded, 1);
    assert_eq!(harness.counters.ingest_snapshot(0).lost_before_capture, 0);
}

#[test]
fn a_tick_says_whether_its_engine_named_the_errored_traces() {
    use sentinel::functions::TraceActivityEventV1;
    let older: TraceActivityEventV1 =
        serde_json::from_value(serde_json::json!({ "trace_ids": ["t1"] })).unwrap();
    assert_eq!(
        older.error_trace_ids, None,
        "an older engine keeps the read"
    );
    let newer: TraceActivityEventV1 = serde_json::from_value(
        serde_json::json!({ "trace_ids": ["t1", "t2"], "error_trace_ids": [] }),
    )
    .unwrap();
    assert_eq!(
        newer.error_trace_ids,
        Some(vec![]),
        "a clean tick queues nothing"
    );
}

#[tokio::test]
async fn an_empty_tick_reads_nothing() {
    let telemetry = FakeEngineTelemetry::default();
    let reads = telemetry.reads.clone();
    let harness = harness(telemetry).await;

    assert!(harness.ingest.filter_tick(&[]).await.0.is_empty());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_tick_becomes_a_group_attributed_to_the_worker_that_owns_the_function() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    let harness = harness(telemetry).await;

    let report = harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(report.recorded, 1, "the cascade is one failure, not three");
    assert!(report.events[0].created);
    assert!(
        report
            .follow_up
            .iter()
            .any(|job| matches!(job, IngestJob::Settle { .. })),
        "a settle pass is scheduled to catch ancestors that were still open"
    );

    assert_eq!(
        one(&harness.store, "SELECT service_name FROM sentinel_groups").await,
        json!("state"),
        "the owner of the function, not the `iii` hop that carried the failure"
    );
    assert_eq!(
        one(&harness.store, "SELECT title FROM sentinel_groups").await,
        json!("CasMismatch: expected version <n>, found <n>")
    );
    assert_eq!(
        one(&harness.store, "SELECT last_version FROM sentinel_groups").await,
        json!("0.23.0")
    );
    assert_eq!(
        one(
            &harness.store,
            "SELECT session_id FROM sentinel_occurrences"
        )
        .await,
        json!("s_7a1"),
        "the session comes from the trace tags"
    );
    assert_eq!(
        count(
            &harness.store,
            "SELECT COUNT(*) FROM sentinel_group_sessions"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn the_settle_pass_refreshes_the_trace_without_re_filing_it() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    let harness = harness(telemetry).await;

    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");
    harness
        .ingest
        .handle(
            IngestJob::Settle {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("settle");

    let evidence = one(&harness.store, "SELECT evidence FROM sentinel_occurrences").await;
    let bundle: Value =
        serde_json::from_str(evidence.as_str().expect("stored")).expect("evidence parses");
    assert_eq!(bundle["settled"], json!(true), "the re-read is recorded");
    assert_eq!(
        bundle["worker"]["service_name"],
        json!("state"),
        "a span names the process that emitted it; the bundle keeps the owner \
         the registry resolved"
    );
    assert_eq!(
        one(&harness.store, "SELECT settled FROM sentinel_occurrences").await,
        json!(1)
    );
}

#[tokio::test]
async fn the_same_span_ticking_twice_is_one_occurrence() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    let harness = harness(telemetry).await;

    // Ticks fire on span open and on span close.
    for _ in 0..2 {
        harness
            .ingest
            .handle(
                IngestJob::Trace {
                    trace_id: "t1".into(),
                },
                &harness.config,
            )
            .await
            .expect("ingest");
    }

    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        1
    );
    assert_eq!(
        one(
            &harness.store,
            "SELECT occurrence_count FROM sentinel_groups"
        )
        .await,
        json!(1)
    );
    assert_eq!(harness.counters.ingest_snapshot(0).deduped, 1);
}

#[tokio::test]
async fn a_trace_this_worker_caused_is_never_ingested() {
    let mut telemetry = FakeEngineTelemetry::default();
    // The sentinel's own queue step, whose `database::execute` failed.
    let db = with_parent(
        span(
            "db",
            "execute database::execute",
            "database",
            "error",
            json!([]),
        ),
        "own",
    );
    let own = span(
        "own",
        "execute sentinel::ingest",
        "sentinel",
        "error",
        json!([db]),
    );
    telemetry.summaries.insert(
        "own".into(),
        summary("own", "sentinel", "sentinel::ingest", &[]),
    );
    telemetry.trees.insert("own".into(), vec![own]);
    let harness = harness(telemetry).await;

    let report = harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "own".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(report.recorded, 0);
    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        0
    );
    assert_eq!(harness.counters.ingest_snapshot(0).dropped_own_trace, 1);
}

#[tokio::test]
async fn an_investigations_own_trace_is_never_ingested() {
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert(
        "inv".into(),
        summary(
            "inv",
            "ide",
            "coder::read-file",
            &[("iii.session.id", "sentinel-inv-01j8m2")],
        ),
    );
    telemetry.trees.insert(
        "inv".into(),
        vec![span(
            "a",
            "execute coder::read-file",
            "ide",
            "error",
            json!([]),
        )],
    );
    let harness = harness(telemetry).await;

    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "inv".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        0
    );
    assert_eq!(harness.counters.ingest_snapshot(0).dropped_investigation, 1);
}

#[tokio::test]
async fn a_trace_that_left_the_ring_before_the_job_ran_is_counted_not_guessed() {
    let harness = harness(FakeEngineTelemetry::default()).await;
    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "gone".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        0
    );
    assert_eq!(
        harness.counters.ingest_snapshot(0).lost_before_capture,
        1,
        "the number that tells an operator to raise the engine's span budget"
    );
}

#[tokio::test]
async fn a_secret_in_a_failure_message_never_reaches_the_store() {
    let mut telemetry = FakeEngineTelemetry::default();
    let mut leaf = span(
        "leaf",
        "execute state::compare-and-set",
        "state",
        "error",
        json!([]),
    );
    leaf["status_description"] = json!("could not connect to postgres://admin:hunter2@db:5432/app");
    telemetry.summaries.insert(
        "t1".into(),
        summary("t1", "state", "state::compare-and-set", &[]),
    );
    telemetry.trees.insert("t1".into(), vec![leaf]);
    let harness = harness(telemetry).await;

    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    let stored = one(&harness.store, "SELECT message FROM sentinel_occurrences").await;
    let evidence = one(&harness.store, "SELECT evidence FROM sentinel_occurrences").await;
    assert!(!stored.to_string().contains("hunter2"), "{stored}");
    assert!(!evidence.to_string().contains("hunter2"), "{evidence}");
    assert!(harness.counters.ingest_snapshot(0).redactions >= 1);
}

#[tokio::test]
async fn a_log_waits_for_the_span_of_its_own_trace_and_then_folds_into_it() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    let harness = harness(telemetry).await;

    let log = json!({
        "timestamp_unix_nano": NOW_NANOS,
        "severity_text": "ERROR",
        "body": "compare-and-set rejected",
        "attributes": { "code.function": "retry_step" },
        "trace_id": "t1",
        "span_id": "leaf",
        "service_name": "harness",
    });

    // The log is emitted inside the handler; the span closes after it.
    let report = harness
        .ingest
        .handle(
            IngestJob::Log {
                trace_id: "t1".into(),
                log: log.clone(),
            },
            &harness.config,
        )
        .await
        .expect("log");
    // Queued at once, these run ahead of the span job behind this log in the
    // trace's queue.
    for job in report.follow_up {
        harness
            .ingest
            .handle(job, &harness.config)
            .await
            .expect("follow-up");
    }

    assert_eq!(
        count(
            &harness.store,
            "SELECT COUNT(*) FROM sentinel_occurrences WHERE pending_join = 1"
        )
        .await,
        1,
        "an ERROR log never becomes a group on arrival"
    );
    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        0
    );

    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(
        count(
            &harness.store,
            "SELECT COUNT(*) FROM sentinel_occurrences WHERE pending_join = 1"
        )
        .await,
        0,
        "the span explains the log, so the placeholder goes"
    );
    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        1,
        "one group, from the span — not a second one from the log"
    );
}

#[tokio::test]
async fn a_log_arriving_after_its_span_becomes_evidence_rather_than_a_group() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary);
    telemetry.trees.insert("t1".into(), tree);
    let harness = harness(telemetry).await;

    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    harness
        .ingest
        .handle(
            IngestJob::Log {
                trace_id: "t1".into(),
                log: json!({
                    "timestamp_unix_nano": NOW_NANOS + 1,
                    "severity_text": "ERROR",
                    "body": "retrying step 2/3",
                    "attributes": {},
                    "trace_id": "t1",
                    "span_id": "leaf",
                    "service_name": "harness",
                }),
            },
            &harness.config,
        )
        .await
        .expect("log");

    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        1
    );
    assert_eq!(
        count(
            &harness.store,
            "SELECT COUNT(*) FROM sentinel_occurrences WHERE pending_join = 1"
        )
        .await,
        0
    );
    let evidence = one(&harness.store, "SELECT evidence FROM sentinel_occurrences").await;
    assert!(
        evidence.to_string().contains("retrying step 2/3"),
        "the log joined the evidence of the failure it belongs to"
    );
}

#[tokio::test]
async fn a_log_whose_span_never_arrives_is_promoted_to_a_group_of_its_own() {
    let mut telemetry = FakeEngineTelemetry::default();
    // The trace exists but never produces an error span: a background loop
    // that logged and carried on.
    telemetry
        .summaries
        .insert("t2".into(), summary("t2", "queue", "queue::deliver", &[]));
    let harness = harness(telemetry).await;

    let arriving = sentinel::ids::now_ms();
    let report = harness
        .ingest
        .handle(
            IngestJob::Log {
                trace_id: "t2".into(),
                log: json!({
                    "timestamp_unix_nano": NOW_NANOS,
                    "severity_text": "ERROR",
                    "body": "redelivery exhausted after 3 attempts",
                    "attributes": { "code.function": "deliver" },
                    "trace_id": "t2",
                    "span_id": "s2",
                    "service_name": "queue",
                }),
            },
            &harness.config,
        )
        .await
        .expect("log");
    let arrived = sentinel::ids::now_ms();

    assert!(
        report.follow_up.is_empty(),
        "nothing promotes the log before its wait runs out"
    );

    let window = harness.config.sources.log.join_window_ms as i64;
    assert!(
        harness
            .store
            .due_pending_logs(arriving + window - 1, 100)
            .await
            .expect("read")
            .is_empty(),
        "the log is still waiting for its span"
    );
    let due = harness
        .store
        .due_pending_logs(arrived + window, 100)
        .await
        .expect("read");
    assert_eq!(due.len(), 1, "the sweep claims it when the wait runs out");

    harness
        .ingest
        .handle(
            IngestJob::Promote {
                trace_id: "t2".into(),
                occurrence_id: due[0].id.clone(),
            },
            &harness.config,
        )
        .await
        .expect("promote");

    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        1
    );
    assert_eq!(
        one(&harness.store, "SELECT source FROM sentinel_groups").await,
        json!("log")
    );
    assert_eq!(
        one(&harness.store, "SELECT title FROM sentinel_groups").await,
        json!("deliver: redelivery exhausted after <n> attempts")
    );
    assert_eq!(
        one(&harness.store, "SELECT last_version FROM sentinel_groups").await,
        json!("git:4662b0d"),
        "a development version is replaced by the commit of the mapped checkout"
    );
    assert_eq!(
        count(
            &harness.store,
            "SELECT COUNT(*) FROM sentinel_occurrences WHERE pending_join = 1"
        )
        .await,
        0,
        "promotion attaches the row it was already holding"
    );
    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_occurrences").await,
        1,
        "and does not insert a second one"
    );
}

/// Ingests an ERROR log stamped `skew_ms` off the real clock; answers the times around its arrival.
async fn log_stamped_off_by(harness: &Harness, skew_ms: i64) -> (i64, i64) {
    let arriving = sentinel::ids::now_ms();
    harness
        .ingest
        .handle(
            IngestJob::Log {
                trace_id: "t2".into(),
                log: json!({
                    "timestamp_unix_nano": (arriving + skew_ms) as u64 * 1_000_000,
                    "severity_text": "ERROR",
                    "body": "redelivery exhausted after 3 attempts",
                    "attributes": {},
                    "trace_id": "t2",
                    "span_id": "s2",
                    "service_name": "queue",
                }),
            },
            &harness.config,
        )
        .await
        .expect("log");
    (arriving, sentinel::ids::now_ms())
}

#[tokio::test]
async fn a_log_from_a_clock_an_hour_ahead_is_due_when_its_window_ends() {
    let harness = harness(FakeEngineTelemetry::default()).await;
    let (_, arrived) = log_stamped_off_by(&harness, 3_600_000).await;

    let window = harness.config.sources.log.join_window_ms as i64;
    let due = harness
        .store
        .due_pending_logs(arrived + window, 100)
        .await
        .expect("read");
    assert_eq!(
        due.len(),
        1,
        "only the sweep promotes a parked log, so a fast clock would hide it for as long as it is fast"
    );
}

#[tokio::test]
async fn a_log_from_a_clock_five_seconds_behind_still_waits_its_window() {
    let harness = harness(FakeEngineTelemetry::default()).await;
    let (arriving, _) = log_stamped_off_by(&harness, -5_000).await;

    let window = harness.config.sources.log.join_window_ms as i64;
    assert!(
        harness
            .store
            .due_pending_logs(arriving + window - 1, 100)
            .await
            .expect("read")
            .is_empty(),
        "a promotion queued now would run ahead of the span job the wait is for"
    );
}

#[tokio::test]
async fn a_second_occurrence_of_a_resolved_group_reopens_it_as_a_regression() {
    let (summary, tree) = cas_trace();
    let mut telemetry = FakeEngineTelemetry::default();
    telemetry.summaries.insert("t1".into(), summary.clone());
    telemetry.trees.insert("t1".into(), tree.clone());
    // The same failure in a second trace.
    let mut later = summary;
    later.trace_id = "t9".into();
    telemetry.summaries.insert("t9".into(), later);
    let mut second_tree = tree;
    rename_spans(&mut second_tree[0]);
    telemetry.trees.insert("t9".into(), second_tree);
    let harness = harness(telemetry).await;

    harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");
    harness
        .store
        .db()
        .execute(
            "UPDATE sentinel_groups SET status = 'resolved', resolved_version = '0.23.0'",
            vec![],
        )
        .await
        .expect("resolve");

    let report = harness
        .ingest
        .handle(
            IngestJob::Trace {
                trace_id: "t9".into(),
            },
            &harness.config,
        )
        .await
        .expect("ingest");

    assert_eq!(report.events[0].status, GroupStatusV1::Regressed);
    assert_eq!(
        count(&harness.store, "SELECT COUNT(*) FROM sentinel_groups").await,
        1,
        "the same failure is the same group"
    );
}

/// Give a tree fresh span ids so it reads as a second occurrence.
fn rename_spans(node: &mut Value) {
    if let Some(span_id) = node.get("span_id").and_then(Value::as_str) {
        node["span_id"] = json!(format!("{span_id}b"));
    }
    if let Some(parent) = node.get("parent_span_id").and_then(Value::as_str) {
        node["parent_span_id"] = json!(format!("{parent}b"));
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            rename_spans(child);
        }
    }
}
