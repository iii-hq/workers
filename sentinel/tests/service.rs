//! The surface the console calls, over a real store.

#[path = "support/sqlite.rs"]
mod sqlite;

use std::sync::Arc;

use async_trait::async_trait;
use sentinel::service::TraceAvailability;
use sentinel::store::Db;
use sentinel::{
    ErrorSourceV1, GroupActionRequestV1, GroupGetRequestV1, GroupStatusV1, GroupsListRequestV1,
    IgnoreRequestV1, IgnoreRuleV1, OccurrenceWrite, OccurrencesListRequestV1, RecordOutcome,
    ResolveRequestV1, Service, Store,
};
use serde_json::{json, Value};
use sqlite::SqliteDb;

const NOW: i64 = 1_789_000_000_000;

struct AlwaysGone;

#[async_trait]
impl TraceAvailability for AlwaysGone {
    async fn trace_exists(&self, _trace_id: &str) -> bool {
        false
    }
}

struct AlwaysThere;

#[async_trait]
impl TraceAvailability for AlwaysThere {
    async fn trace_exists(&self, _trace_id: &str) -> bool {
        true
    }
}

async fn fixture(traces: Arc<dyn TraceAvailability>) -> (Arc<Store<SqliteDb>>, Service<SqliteDb>) {
    let store = Arc::new(Store::new(SqliteDb::in_memory()));
    store.migrate().await.expect("migrate");
    let service = Service::new(store.clone(), traces);
    (store, service)
}

fn write(fingerprint: &str, dedupe: &str, service_name: &str, at_ms: i64) -> OccurrenceWrite {
    OccurrenceWrite {
        fingerprint: fingerprint.into(),
        source: ErrorSourceV1::Trace,
        dedupe_key: dedupe.into(),
        at_ms,
        namespace: "default".into(),
        service_name: service_name.into(),
        function_id: Some(format!("{service_name}::work")),
        exception_type: Some("CasMismatch".into()),
        title: format!("CasMismatch: {service_name} failed"),
        message: "expected version 41, found 42".into(),
        trace_id: Some(format!("trace-{dedupe}")),
        span_id: Some("span".into()),
        session_id: Some(format!("s_{service_name}")),
        turn_id: None,
        worker_version: Some("0.23.0".into()),
        evidence: Some(r#"{"version":1,"captured_at_ms":1,"settled":false,"trace_id":"t","origin_span_id":"s","propagated_through":[],"trace_tags":{},"spans":[],"logs":[],"worker":{"service_name":"harness"},"truncated":{"spans":0,"logs":0,"attributes":0}}"#.into()),
        namespace_ambiguous: false,
        pending_occurrence_id: None,
    }
}

async fn create(store: &Store<SqliteDb>, write: &OccurrenceWrite) -> String {
    match store.record_occurrence(write).await.expect("record") {
        RecordOutcome::Created { group_id } => group_id,
        other => panic!("expected a new group, got {other:?}"),
    }
}

async fn set_status(store: &Store<SqliteDb>, group_id: &str, sql: &str) {
    store
        .db()
        .execute(
            &format!("UPDATE sentinel_groups SET {sql} WHERE id = ?"),
            vec![json!(group_id)],
        )
        .await
        .expect("update");
}

#[tokio::test]
async fn the_list_puts_regressions_first_because_they_are_the_news() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let old = create(&store, &write("fp_old", "a", "queue", NOW)).await;
    let recent = create(&store, &write("fp_recent", "b", "state", NOW + 10_000)).await;
    let regressed = create(
        &store,
        &write("fp_regressed", "c", "harness", NOW - 100_000),
    )
    .await;
    set_status(
        &store,
        &regressed,
        &format!("status = 'regressed', regressed_at_ms = {}", NOW - 1),
    )
    .await;

    let listed = service
        .list(GroupsListRequestV1::default())
        .await
        .expect("list");
    let ids: Vec<&str> = listed.groups.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids[0], regressed, "a failure that came back leads: {ids:?}");
    assert_eq!(ids[1], recent, "then the most recent");
    assert_eq!(ids[2], old);
    assert_eq!(listed.total, 3);
}

#[tokio::test]
async fn the_default_list_is_the_open_states_and_filters_narrow_it() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let open = create(&store, &write("fp_open", "a", "queue", NOW)).await;
    let resolved = create(&store, &write("fp_resolved", "b", "state", NOW)).await;
    set_status(&store, &resolved, "status = 'resolved'").await;

    let default = service
        .list(GroupsListRequestV1::default())
        .await
        .expect("list");
    assert_eq!(default.groups.len(), 1);
    assert_eq!(default.groups[0].id, open);

    let closed = service
        .list(GroupsListRequestV1 {
            status: Some(vec![GroupStatusV1::Resolved]),
            ..GroupsListRequestV1::default()
        })
        .await
        .expect("list");
    assert_eq!(closed.groups[0].id, resolved);

    let by_worker = service
        .list(GroupsListRequestV1 {
            service_name: Some("queue".into()),
            ..GroupsListRequestV1::default()
        })
        .await
        .expect("list");
    assert_eq!(by_worker.groups.len(), 1);
    assert_eq!(by_worker.groups[0].service_name, "queue");

    let searched = service
        .list(GroupsListRequestV1 {
            search: Some("queue::work".into()),
            ..GroupsListRequestV1::default()
        })
        .await
        .expect("list");
    assert_eq!(searched.groups.len(), 1);

    let nothing = service
        .list(GroupsListRequestV1 {
            search: Some("nothing matches this".into()),
            ..GroupsListRequestV1::default()
        })
        .await
        .expect("list");
    assert!(nothing.groups.is_empty());
}

#[tokio::test]
async fn a_summary_carries_the_counts_the_row_cannot_hold() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;
    let mut second = write("fp", "b", "queue", NOW + 1_000);
    second.session_id = Some("s_other".into());
    store.record_occurrence(&second).await.expect("record");

    let listed = service
        .list(GroupsListRequestV1::default())
        .await
        .expect("list");
    let summary = &listed.groups[0];
    assert_eq!(summary.id, group);
    assert_eq!(summary.occurrence_count, 2);
    assert_eq!(
        summary.sessions_affected, 2,
        "counted from its own table, so retention cannot make it lie"
    );
    assert_eq!(summary.sparkline.len(), 24, "one bar per hour of the day");
}

#[tokio::test]
async fn the_detail_says_whether_the_trace_is_still_readable() {
    let (store, gone) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;

    let detail = gone.get(GroupGetRequestV1::new(&group)).await.expect("get");
    assert_eq!(detail.group.id, group);
    assert_eq!(detail.message_sample, "expected version 41, found 42");
    assert!(detail.latest_occurrence.is_some());
    assert!(
        !detail.trace_available,
        "the engine's ring dropped it: the snapshot is all there is"
    );

    let there = Service::new(store.clone(), Arc::new(AlwaysThere));
    let detail = there
        .get(GroupGetRequestV1::new(&group))
        .await
        .expect("get");
    assert!(detail.trace_available);
}

#[tokio::test]
async fn occurrences_page_newest_first_and_report_their_evidence() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;
    store
        .record_occurrence(&write("fp", "b", "queue", NOW + 1_000))
        .await
        .expect("record");

    let page = service
        .occurrences(OccurrencesListRequestV1 {
            group_id: group.clone(),
            limit: Some(1),
            ..OccurrencesListRequestV1::default()
        })
        .await
        .expect("occurrences");
    assert_eq!(page.total, 2);
    assert_eq!(page.occurrences.len(), 1);
    assert_eq!(page.occurrences[0].at_ms, NOW + 1_000);
    assert!(page.occurrences[0].has_evidence);
}

#[tokio::test]
async fn a_pruned_bundle_reads_as_pruned_rather_than_missing() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;
    let occurrence_id = service
        .occurrences(OccurrencesListRequestV1 {
            group_id: group,
            ..OccurrencesListRequestV1::default()
        })
        .await
        .expect("occurrences")
        .occurrences
        .remove(0)
        .id;

    let kept = service
        .evidence(sentinel::EvidenceGetRequestV1 {
            occurrence_id: occurrence_id.clone(),
            ..Default::default()
        })
        .await
        .expect("evidence");
    assert!(kept.evidence.is_some());
    assert!(!kept.pruned);

    store
        .db()
        .execute(
            "UPDATE sentinel_occurrences SET evidence = NULL WHERE id = ?",
            vec![json!(occurrence_id)],
        )
        .await
        .expect("prune");

    let pruned = service
        .evidence(sentinel::EvidenceGetRequestV1 {
            occurrence_id,
            ..Default::default()
        })
        .await
        .expect("evidence");
    assert!(pruned.pruned, "the row stays and says the bundle went");
    assert!(pruned.evidence.is_none());
}

#[tokio::test]
async fn resolving_records_the_version_it_was_resolved_on() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;

    let response = service
        .resolve(ResolveRequestV1 {
            group_id: group.clone(),
            until_version_change: true,
            ..Default::default()
        })
        .await
        .expect("resolve");
    assert_eq!(response.status, GroupStatusV1::Resolved);

    let row = store
        .db()
        .query(
            "SELECT resolved_version, resolve_until_version_change FROM sentinel_groups WHERE id = ?",
            vec![json!(group)],
        )
        .await
        .expect("query");
    assert_eq!(row[0]["resolved_version"], json!("0.23.0"));
    assert_eq!(row[0]["resolve_until_version_change"], json!(1));
}

#[tokio::test]
async fn ignoring_captures_its_baseline_from_the_moment_somebody_said_so() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;
    store
        .record_occurrence(&write("fp", "b", "queue", NOW + 1))
        .await
        .expect("record");

    service
        .ignore(IgnoreRequestV1 {
            group_id: group.clone(),
            rule: IgnoreRuleV1::Occurrences { count: 50 },
            _caller_worker_id: None,
        })
        .await
        .expect("ignore");

    let row = store
        .db()
        .query(
            "SELECT ignore_baseline FROM sentinel_groups WHERE id = ?",
            vec![json!(group)],
        )
        .await
        .expect("query");
    let baseline: Value =
        serde_json::from_str(row[0]["ignore_baseline"].as_str().expect("stored")).expect("json");
    assert_eq!(
        baseline["occurrence_count"],
        json!(2),
        "50 more counts from now, not from the group's first occurrence"
    );
}

#[tokio::test]
async fn reopening_clears_everything_the_closing_decision_set() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;
    service
        .resolve(ResolveRequestV1 {
            group_id: group.clone(),
            until_version_change: true,
            ..Default::default()
        })
        .await
        .expect("resolve");

    let reopened = service
        .reopen(GroupActionRequestV1::new(&group))
        .await
        .expect("reopen");
    assert_eq!(reopened.status, GroupStatusV1::New);

    let row = store
        .db()
        .query(
            "SELECT resolved_version, resolve_until_version_change, ignore_rule \
             FROM sentinel_groups WHERE id = ?",
            vec![json!(group)],
        )
        .await
        .expect("query");
    assert_eq!(row[0]["resolved_version"], Value::Null);
    assert_eq!(row[0]["resolve_until_version_change"], json!(0));
    assert_eq!(row[0]["ignore_rule"], Value::Null);
}

#[tokio::test]
async fn a_decision_the_lifecycle_refuses_comes_back_as_an_invalid_transition() {
    let (store, service) = fixture(Arc::new(AlwaysGone)).await;
    let group = create(&store, &write("fp", "a", "queue", NOW)).await;
    set_status(&store, &group, "status = 'investigating'").await;

    let error = service
        .resolve(ResolveRequestV1 {
            group_id: group.clone(),
            ..Default::default()
        })
        .await
        .expect_err("refused while a first pass is running");
    assert_eq!(error.code(), "invalid_transition");

    let missing = service
        .unignore(GroupActionRequestV1::new("grp_nope"))
        .await
        .expect_err("no such group");
    assert_eq!(missing.code(), "not_found");
}
