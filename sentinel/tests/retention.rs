//! What the store lets go of, and what it refuses to.
//!
//! The counters on a group never shrink — an incident's size is part of what
//! it means — but the frozen bundles behind it do. What must survive is what
//! somebody will ask for months later: how it started, how it looks now, and
//! how it looked on each version in between.

#[path = "support/sqlite.rs"]
mod sqlite;

use std::sync::Arc;

use sentinel::store::Db;
use sentinel::{
    ErrorSourceV1, IgnoreRequestV1, IgnoreRuleV1, OccurrenceWrite, RecordOutcome, Store,
    WorkerConfig,
};
use serde_json::{json, Value};
use sqlite::SqliteDb;

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;

async fn store() -> Arc<Store<SqliteDb>> {
    let store = Arc::new(Store::new(SqliteDb::in_memory()));
    store.migrate().await.expect("migrate");
    store
}

fn write(dedupe: &str, at_ms: i64, version: &str) -> OccurrenceWrite {
    OccurrenceWrite {
        fingerprint: "fp-one".into(),
        source: ErrorSourceV1::Trace,
        dedupe_key: dedupe.into(),
        at_ms,
        namespace: "default".into(),
        service_name: "compose".into(),
        function_id: Some("compose::operation".into()),
        exception_type: Some("UnknownOperation".into()),
        title: "UnknownOperation: unknown compose operation".into(),
        message: "unknown compose operation `<str>`".into(),
        trace_id: Some(format!("t-{dedupe}")),
        span_id: Some("s1".into()),
        session_id: Some("s_user".into()),
        turn_id: None,
        worker_version: Some(version.into()),
        evidence: Some(format!(
            r#"{{"version":1,"captured_at_ms":{at_ms},"settled":true,"trace_id":"t","origin_span_id":"s","propagated_through":[],"trace_tags":{{}},"spans":[],"logs":[],"worker":{{"service_name":"compose"}},"truncated":{{"spans":0,"logs":0,"attributes":0}}}}"#
        )),
        namespace_ambiguous: false,
        pending_occurrence_id: None,
    }
}

/// Twelve occurrences an hour apart, the oldest four on an older version.
async fn seed(store: &Store<SqliteDb>) -> String {
    let mut group_id = String::new();
    for index in 0..12 {
        let version = if index < 4 { "0.23.0" } else { "0.24.0" };
        let at_ms = NOW - (12 - index) * 3_600_000;
        let outcome = store
            .record_occurrence(&write(&format!("trace:t{index}:s1"), at_ms, version))
            .await
            .expect("record");
        if let RecordOutcome::Created { group_id: id } = outcome {
            group_id = id;
        }
    }
    group_id
}

async fn kept(store: &Store<SqliteDb>, group_id: &str) -> Vec<(i64, String)> {
    store
        .db()
        .query(
            "SELECT at_ms, worker_version FROM sentinel_occurrences \
             WHERE group_id = ? AND evidence IS NOT NULL ORDER BY at_ms ASC",
            vec![json!(group_id)],
        )
        .await
        .expect("read")
        .iter()
        .map(|row| {
            (
                row.get("at_ms").and_then(Value::as_i64).unwrap_or_default(),
                row.get("worker_version")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn pruning_keeps_the_beginning_the_present_and_one_of_every_version() {
    let store = store().await;
    let group_id = seed(&store).await;
    let mut config = WorkerConfig::default();
    config.retention.evidence_per_group = 3;

    sentinel::retention::prune_group(&store, &config, &group_id)
        .await
        .expect("prune");

    let kept = kept(&store, &group_id).await;
    let first = NOW - 12 * 3_600_000;
    assert_eq!(
        kept.first().map(|(at_ms, _)| *at_ms),
        Some(first),
        "the group's own beginning is worth more than any later sample: {kept:?}"
    );
    assert_eq!(
        kept.last().map(|(at_ms, _)| *at_ms),
        Some(NOW - 3_600_000),
        "and the most recent is what somebody opens first: {kept:?}"
    );
    let versions: Vec<&str> = kept.iter().map(|(_, version)| version.as_str()).collect();
    assert!(
        versions.contains(&"0.23.0") && versions.contains(&"0.24.0"),
        "comparing the failure across a release is the point of keeping anything: {kept:?}"
    );
    assert!(
        kept.len() <= 5,
        "three recent, the first, and one per version — not the whole history: {kept:?}"
    );

    let rows: i64 = store
        .db()
        .query(
            "SELECT COUNT(*) AS total FROM sentinel_occurrences WHERE group_id = ?",
            vec![json!(group_id)],
        )
        .await
        .expect("count")
        .first()
        .and_then(|row| row.get("total"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    assert_eq!(rows, 12, "the rows stay; only the bundles go");

    let count = store
        .group_by_id(&group_id)
        .await
        .expect("read")
        .expect("exists")
        .state
        .occurrence_count;
    assert_eq!(count, 12, "an incident's size is part of what it means");
}

#[tokio::test]
async fn an_ignored_group_keeps_only_what_it_takes_to_look_at_it_once() {
    let store = store().await;
    let group_id = seed(&store).await;
    let service = sentinel::Service::new(store.clone(), Arc::new(Gone));
    service
        .ignore(IgnoreRequestV1 {
            group_id: group_id.clone(),
            rule: IgnoreRuleV1::Forever,
            _caller_worker_id: None,
        })
        .await
        .expect("ignore");

    sentinel::retention::prune_group(&store, &WorkerConfig::default(), &group_id)
        .await
        .expect("prune");

    let kept = kept(&store, &group_id).await;
    assert!(
        kept.len() <= 3,
        "somebody asked not to hear about this one: {kept:?}"
    );
    assert_eq!(
        kept.last().map(|(at_ms, _)| *at_ms),
        Some(NOW - 3_600_000),
        "the latest is still there for the moment they change their mind"
    );
}

#[tokio::test]
async fn the_daily_pass_drops_old_buckets_and_archives_quiet_resolved_groups() {
    let store = store().await;
    let group_id = seed(&store).await;
    let mut config = WorkerConfig::default();
    config.retention.buckets_days = 7;
    config.retention.resolved_ttl_days = 30;

    // One bucket inside the window and one long outside it.
    for hour_ms in [NOW - 2 * DAY, NOW - 40 * DAY] {
        store
            .db()
            .execute(
                "INSERT INTO sentinel_buckets (group_id, hour_ms, count) VALUES (?, ?, 1)",
                vec![json!(group_id), json!(hour_ms)],
            )
            .await
            .expect("seed a bucket");
    }
    // A group resolved long ago and quiet since.
    store
        .db()
        .execute(
            "UPDATE sentinel_groups SET status = 'resolved', last_seen_ms = ?, \
             resolved_at_ms = ? WHERE id = ?",
            vec![
                json!(NOW - 60 * DAY),
                json!(NOW - 60 * DAY),
                json!(group_id),
            ],
        )
        .await
        .expect("resolve it in the past");

    let outcome = sentinel::retention::prune(&store, &config)
        .await
        .expect("prune");

    assert_eq!(
        outcome.buckets_removed, 1,
        "only the one outside the window"
    );
    assert_eq!(outcome.groups_archived, 1);
    assert!(outcome.evidence_pruned > 0);
    assert!(
        kept(&store, &group_id).await.is_empty(),
        "an archived group keeps its counts and lets its bundles go"
    );

    let archived = store
        .db()
        .query(
            "SELECT archived, status, occurrence_count FROM sentinel_groups WHERE id = ?",
            vec![json!(group_id)],
        )
        .await
        .expect("read");
    let row = archived.first().expect("exists");
    assert_eq!(row.get("archived").and_then(Value::as_i64), Some(1));
    assert_eq!(
        row.get("occurrence_count").and_then(Value::as_i64),
        Some(12),
        "the number of times it happened is not retention's to forget"
    );
}

#[tokio::test]
async fn a_resolved_group_still_being_hit_is_not_archived() {
    let store = store().await;
    let group_id = seed(&store).await;
    let config = WorkerConfig::default();
    store
        .db()
        .execute(
            "UPDATE sentinel_groups SET status = 'resolved' WHERE id = ?",
            vec![json!(group_id)],
        )
        .await
        .expect("resolve it");

    let outcome = sentinel::retention::prune(&store, &config)
        .await
        .expect("prune");
    assert_eq!(
        outcome.groups_archived, 0,
        "it was resolved, but it is still happening"
    );
    assert!(!kept(&store, &group_id).await.is_empty());
}

#[tokio::test]
async fn the_row_ceiling_drops_the_oldest_but_never_the_first() {
    let store = store().await;
    let group_id = seed(&store).await;
    let mut config = WorkerConfig::default();
    config.retention.occurrences_per_group = 4;

    sentinel::retention::prune_group(&store, &config, &group_id)
        .await
        .expect("prune");

    let rows = store
        .db()
        .query(
            "SELECT at_ms FROM sentinel_occurrences WHERE group_id = ? ORDER BY at_ms ASC",
            vec![json!(group_id)],
        )
        .await
        .expect("read");
    let times: Vec<i64> = rows
        .iter()
        .filter_map(|row| row.get("at_ms").and_then(Value::as_i64))
        .collect();
    assert_eq!(times.len(), 5, "four recent plus the first: {times:?}");
    assert_eq!(times[0], NOW - 12 * 3_600_000);
    assert_eq!(times[1], NOW - 4 * 3_600_000);

    let count = store
        .group_by_id(&group_id)
        .await
        .expect("read")
        .expect("exists")
        .state
        .occurrence_count;
    assert_eq!(count, 12);
}

struct Gone;

#[async_trait::async_trait]
impl sentinel::service::TraceAvailability for Gone {
    async fn trace_exists(&self, _trace_id: &str) -> bool {
        false
    }
}
