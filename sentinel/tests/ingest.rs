//! The ingest pipeline end to end, over a real SQLite store.
//!
//! The engine is faked because its telemetry is what we are simulating; the
//! store is not, because what these tests are really checking is that a tick
//! becomes the right row.

#[path = "support/sqlite.rs"]
mod sqlite;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sentinel::ingest::{ring::PhantomRing, CheckoutVersions, Ingest, IngestJob, Telemetry};
use sentinel::registry::{EngineRegistry, FunctionEntry, Registry, WorkerEntry};
use sentinel::store::Db;
use sentinel::{Counters, GroupStatusV1, SentinelError, Store, TraceSummary, WorkerConfig};
use serde_json::{json, Value};
use sqlite::SqliteDb;

const NOW_NANOS: u64 = 1_789_000_000_500_000_000;

#[derive(Default)]
struct FakeEngineTelemetry {
    summaries: BTreeMap<String, TraceSummary>,
    trees: BTreeMap<String, Vec<Value>>,
    logs: BTreeMap<String, Vec<Value>>,
}

#[async_trait]
impl Telemetry for FakeEngineTelemetry {
    async fn traces(&self, trace_ids: &[String]) -> Result<Vec<TraceSummary>, SentinelError> {
        Ok(trace_ids
            .iter()
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
    ingest: Ingest<SqliteDb, FakeRegistry>,
    store: Arc<Store<SqliteDb>>,
    counters: Arc<Counters>,
    config: WorkerConfig,
}

async fn harness(telemetry: FakeEngineTelemetry) -> Harness {
    let store = Arc::new(Store::new(SqliteDb::in_memory()));
    store.migrate().await.expect("migrate");
    let counters = Arc::new(Counters::default());
    let ingest = Ingest::new(
        store.clone(),
        Arc::new(telemetry),
        Arc::new(Registry::new(FakeRegistry, Duration::from_secs(60))),
        Arc::new(FakeCheckouts),
        counters.clone(),
        Arc::new(PhantomRing::default()),
    );
    Harness {
        ingest,
        store,
        counters,
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

async fn count(store: &Store<SqliteDb>, sql: &str) -> i64 {
    store
        .db()
        .query(sql, vec![])
        .await
        .expect("query")
        .first()
        .and_then(|row| row.values().next().cloned())
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

async fn one(store: &Store<SqliteDb>, sql: &str) -> Value {
    store
        .db()
        .query(sql, vec![])
        .await
        .expect("query")
        .first()
        .and_then(|row| row.values().next().cloned())
        .unwrap_or(Value::Null)
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
    harness
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

    let promote = report
        .follow_up
        .into_iter()
        .find_map(|job| match job {
            IngestJob::Promote { occurrence_id, .. } => Some(occurrence_id),
            _ => None,
        })
        .expect("the wait is scheduled");

    harness
        .ingest
        .handle(
            IngestJob::Promote {
                trace_id: "t2".into(),
                occurrence_id: promote,
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
