//! The store's statements, against a real SQLite.
//!
//! These are the tests that would have caught the data-model defects the
//! adversarial review found: a pending log needs a row before its group
//! exists, and a group's transition has to survive somebody else writing
//! between the read and the write.

#[path = "support/sqlite.rs"]
mod sqlite;

use sentinel::store::schema::SCHEMA_VERSION;
use sentinel::{
    Db, ErrorSourceV1, GroupStatusV1, IgnoreBaselineV1, IgnoreRuleV1, OccurrenceWrite,
    RecordOutcome, Statement, Store,
};
use serde_json::{json, Value};
use sqlite::SqliteDb;

const NOW: i64 = 1_789_000_000_000;

async fn store() -> Store<SqliteDb> {
    let store = Store::new(SqliteDb::in_memory());
    store.migrate().await.expect("migrate a fresh database");
    store
}

fn write(dedupe: &str, at_ms: i64) -> OccurrenceWrite {
    OccurrenceWrite {
        fingerprint: "fp_cas".into(),
        source: ErrorSourceV1::Trace,
        dedupe_key: dedupe.into(),
        at_ms,
        namespace: "default".into(),
        service_name: "harness".into(),
        function_id: Some("state::compare-and-set".into()),
        exception_type: Some("CasMismatch".into()),
        title: "CasMismatch: expected version <n>, found <n>".into(),
        message: "expected version 41, found 42".into(),
        trace_id: Some("3f9a1c2b".into()),
        span_id: Some("c21e9b8a".into()),
        session_id: Some("s_7a1".into()),
        turn_id: Some("t_12".into()),
        worker_version: Some("0.23.0".into()),
        evidence: Some(r#"{"version":1}"#.into()),
        namespace_ambiguous: false,
        pending_occurrence_id: None,
    }
}

async fn column(store: &Store<SqliteDb>, sql: &str, params: Vec<Value>) -> Value {
    let rows = store.db().query(sql, params).await.expect("query");
    rows.first()
        .and_then(|row| row.values().next().cloned())
        .unwrap_or(Value::Null)
}

async fn set_group(store: &Store<SqliteDb>, group_id: &str, sql: &str, params: Vec<Value>) {
    let mut params = params;
    params.push(json!(group_id));
    store
        .db()
        .execute(
            &format!("UPDATE sentinel_groups SET {sql} WHERE id = ?"),
            params,
        )
        .await
        .expect("update the group");
}

#[tokio::test]
async fn migrating_is_idempotent_and_records_its_version() {
    let store = store().await;
    assert_eq!(store.schema_version().await.unwrap(), SCHEMA_VERSION);
    // Boot runs this every time.
    assert_eq!(store.migrate().await.unwrap(), SCHEMA_VERSION);
    assert_eq!(store.schema_version().await.unwrap(), SCHEMA_VERSION);
}

#[tokio::test]
async fn the_first_occurrence_creates_its_group_with_a_session_and_a_bucket() {
    let store = store().await;
    let outcome = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap();
    let RecordOutcome::Created { group_id } = outcome else {
        panic!("expected a new group, got {outcome:?}");
    };

    let group = store
        .group_by_id(&group_id)
        .await
        .unwrap()
        .expect("the group");
    assert_eq!(group.state.status, GroupStatusV1::New);
    assert_eq!(group.state.occurrence_count, 1);

    assert_eq!(
        column(
            &store,
            "SELECT COUNT(*) FROM sentinel_group_sessions WHERE group_id = ?",
            vec![json!(group_id)]
        )
        .await,
        json!(1)
    );
    assert_eq!(
        column(
            &store,
            "SELECT count FROM sentinel_buckets WHERE group_id = ?",
            vec![json!(group_id)]
        )
        .await,
        json!(1),
        "the sparkline bucket is written with the occurrence, not derived later"
    );
    assert_eq!(
        column(
            &store,
            "SELECT evidence_bytes FROM sentinel_occurrences WHERE dedupe_key = ?",
            vec![json!("trace:a:1")]
        )
        .await,
        json!(13)
    );
}

#[tokio::test]
async fn the_same_physical_event_is_recorded_once() {
    let store = store().await;
    let first = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap();
    let RecordOutcome::Created { group_id } = first else {
        panic!("expected a new group");
    };

    // A span ticks on open and on close: the same span arrives twice.
    let second = store
        .record_occurrence(&write("trace:a:1", NOW + 40))
        .await
        .unwrap();
    assert!(
        matches!(second, RecordOutcome::Deduped { .. }),
        "{second:?}"
    );

    let group = store.group_by_id(&group_id).await.unwrap().unwrap();
    assert_eq!(
        group.state.occurrence_count, 1,
        "a duplicate must not count"
    );
}

#[tokio::test]
async fn a_second_occurrence_counts_against_the_same_group() {
    let store = store().await;
    store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap();
    let outcome = store
        .record_occurrence(&write("trace:b:1", NOW + 3_600_000))
        .await
        .unwrap();

    let RecordOutcome::Counted {
        group_id,
        status,
        changed,
    } = outcome
    else {
        panic!("expected a count, got {outcome:?}");
    };
    assert_eq!(status, GroupStatusV1::New);
    assert!(!changed, "counting is not a state change");

    let group = store.group_by_id(&group_id).await.unwrap().unwrap();
    assert_eq!(group.state.occurrence_count, 2);
    assert_eq!(
        column(
            &store,
            "SELECT COUNT(*) FROM sentinel_buckets WHERE group_id = ?",
            vec![json!(group_id)]
        )
        .await,
        json!(2),
        "an hour later is a second bucket"
    );
}

#[tokio::test]
async fn an_occurrence_after_a_resolve_reopens_the_group_as_a_regression() {
    let store = store().await;
    let RecordOutcome::Created { group_id } = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap()
    else {
        panic!("expected a new group");
    };
    set_group(
        &store,
        &group_id,
        "status = 'resolved', resolved_version = '0.23.0', resolved_at_ms = ?",
        vec![json!(NOW)],
    )
    .await;

    let outcome = store
        .record_occurrence(&write("trace:b:1", NOW + 1_000))
        .await
        .unwrap();
    assert!(
        matches!(
            &outcome,
            RecordOutcome::Counted {
                status: GroupStatusV1::Regressed,
                changed: true,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        column(
            &store,
            "SELECT regressed_at_ms FROM sentinel_groups WHERE id = ?",
            vec![json!(group_id)]
        )
        .await,
        json!(NOW + 1_000),
        "the regression timestamp is what floats the group to the top of the list"
    );
}

#[tokio::test]
async fn resolve_until_version_change_keeps_counting_on_the_version_it_resolved() {
    let store = store().await;
    let RecordOutcome::Created { group_id } = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap()
    else {
        panic!("expected a new group");
    };
    set_group(
        &store,
        &group_id,
        "status = 'resolved', resolved_version = '0.23.0', resolve_until_version_change = 1",
        vec![],
    )
    .await;

    let same_version = store
        .record_occurrence(&write("trace:b:1", NOW + 1_000))
        .await
        .unwrap();
    assert!(
        matches!(
            &same_version,
            RecordOutcome::Counted {
                status: GroupStatusV1::Resolved,
                ..
            }
        ),
        "the fix is not deployed here yet: {same_version:?}"
    );

    let mut shipped = write("trace:c:1", NOW + 2_000);
    shipped.worker_version = Some("0.23.1".into());
    let after_deploy = store.record_occurrence(&shipped).await.unwrap();
    assert!(
        matches!(
            &after_deploy,
            RecordOutcome::Counted {
                status: GroupStatusV1::Regressed,
                ..
            }
        ),
        "{after_deploy:?}"
    );
}

#[tokio::test]
async fn an_expiring_ignore_returns_the_group_and_clears_its_rule() {
    let store = store().await;
    let RecordOutcome::Created { group_id } = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap()
    else {
        panic!("expected a new group");
    };
    let rule = serde_json::to_string(&IgnoreRuleV1::Occurrences { count: 2 }).unwrap();
    let baseline = serde_json::to_string(&IgnoreBaselineV1 {
        occurrence_count: 1,
        worker_version: Some("0.23.0".into()),
    })
    .unwrap();
    set_group(
        &store,
        &group_id,
        "status = 'ignored', ignore_rule = ?, ignore_baseline = ?",
        vec![json!(rule), json!(baseline)],
    )
    .await;

    let still_ignored = store
        .record_occurrence(&write("trace:b:1", NOW + 1_000))
        .await
        .unwrap();
    assert!(
        matches!(
            &still_ignored,
            RecordOutcome::Counted {
                status: GroupStatusV1::Ignored,
                ..
            }
        ),
        "one of two: {still_ignored:?}"
    );

    let reopened = store
        .record_occurrence(&write("trace:c:1", NOW + 2_000))
        .await
        .unwrap();
    assert!(
        matches!(
            &reopened,
            RecordOutcome::Counted {
                status: GroupStatusV1::New,
                changed: true,
                ..
            }
        ),
        "{reopened:?}"
    );
    assert_eq!(
        column(
            &store,
            "SELECT ignore_rule FROM sentinel_groups WHERE id = ?",
            vec![json!(group_id)]
        )
        .await,
        Value::Null,
        "an expired rule is cleared, not left to re-fire"
    );
}

#[tokio::test]
async fn a_compare_and_set_that_loses_the_race_is_retried_against_the_new_state() {
    let store = store().await;
    let RecordOutcome::Created { group_id } = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap()
    else {
        panic!("expected a new group");
    };

    // Simulate the real race: the snapshot the writer read is already stale
    // because a person resolved the group a moment ago.
    let stale = store.group_by_id(&group_id).await.unwrap().unwrap();
    set_group(
        &store,
        &group_id,
        "status = 'resolved', resolved_version = '0.23.0', updated_ms = ?",
        vec![json!(NOW + 500)],
    )
    .await;
    assert_ne!(
        stale.updated_ms,
        column(
            &store,
            "SELECT updated_ms FROM sentinel_groups WHERE id = ?",
            vec![json!(group_id)]
        )
        .await
        .as_i64()
        .unwrap(),
        "the snapshot is stale"
    );

    // The record path re-reads and lands on the current state instead of
    // overwriting it with the decision it made from the old one.
    let outcome = store
        .record_occurrence(&write("trace:b:1", NOW + 1_000))
        .await
        .unwrap();
    assert!(
        matches!(
            &outcome,
            RecordOutcome::Counted {
                status: GroupStatusV1::Regressed,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_occurrence_without_a_group_is_only_legal_while_it_waits_for_its_span() {
    let store = store().await;
    let pending = Statement::new(
        "INSERT INTO sentinel_occurrences (id, group_id, dedupe_key, source, at_ms, trace_id, \
         message, pending_join, join_deadline_ms) VALUES (?, NULL, ?, 'log', ?, ?, ?, 1, ?)",
        vec![
            json!("occ_pending"),
            json!("log:t1:-:1:abc"),
            json!(NOW),
            json!("t1"),
            json!("redelivery exhausted"),
            json!(NOW + 2_000),
        ],
    );
    store
        .db()
        .transaction(&[pending])
        .await
        .expect("a log may wait for its span without a group");

    let orphan = Statement::new(
        "INSERT INTO sentinel_occurrences (id, group_id, dedupe_key, source, at_ms, message, \
         pending_join) VALUES (?, NULL, ?, 'log', ?, ?, 0)",
        vec![
            json!("occ_orphan"),
            json!("log:t2:-:1:def"),
            json!(NOW),
            json!("orphan"),
        ],
    );
    let error = store
        .db()
        .transaction(&[orphan])
        .await
        .expect_err("a settled occurrence must belong to a group");
    assert!(
        error.to_string().to_lowercase().contains("check"),
        "{error}"
    );
}

#[tokio::test]
async fn only_one_first_pass_can_run_per_group() {
    let store = store().await;
    let RecordOutcome::Created { group_id } = store
        .record_occurrence(&write("trace:a:1", NOW))
        .await
        .unwrap()
    else {
        panic!("expected a new group");
    };

    let investigation = |id: &str, status: &str| {
        Statement::new(
            "INSERT INTO sentinel_investigations (id, group_id, occurrence_id, session_id, mode, \
             model, status, created_ms) VALUES (?, ?, 'occ_1', ?, 'assisted', 'm', ?, ?)",
            vec![
                json!(id),
                json!(group_id),
                json!(format!("sentinel-inv-{id}")),
                json!(status),
                json!(NOW),
            ],
        )
    };

    store
        .db()
        .transaction(&[investigation("inv_1", "running")])
        .await
        .expect("the first pass starts");
    store
        .db()
        .transaction(&[investigation("inv_2", "running")])
        .await
        .expect_err("a second concurrent first pass is refused by the store itself");
    store
        .db()
        .transaction(&[investigation("inv_3", "completed")])
        .await
        .expect("a finished investigation does not hold the slot");
}

#[tokio::test]
async fn status_counts_group_by_state_and_treat_regressed_as_open() {
    let store = store().await;
    for (index, status) in [
        "new",
        "investigating",
        "diagnosed",
        "regressed",
        "ignored",
        "resolved",
    ]
    .into_iter()
    .enumerate()
    {
        let mut occurrence = write(&format!("trace:{index}:1"), NOW + index as i64);
        occurrence.fingerprint = format!("fp_{index}");
        let RecordOutcome::Created { group_id } =
            store.record_occurrence(&occurrence).await.unwrap()
        else {
            panic!("expected a new group");
        };
        set_group(&store, &group_id, "status = ?", vec![json!(status)]).await;
    }

    let counts = store.group_counts().await.unwrap();
    assert_eq!(
        counts.open, 4,
        "new, investigating, diagnosed and regressed"
    );
    assert_eq!(counts.regressed, 1);
    assert_eq!(counts.ignored, 1);
    assert_eq!(counts.resolved, 1);
}

#[tokio::test]
async fn a_store_already_at_v1_takes_the_upgrade() {
    // The real path on every existing install: the tables are there, the
    // meta row says 1, and boot has to add v2 without touching the rest.
    let store = Store::new(SqliteDb::in_memory());
    let db = store.db();
    db.execute(sentinel::store::schema::META_TABLE, vec![])
        .await
        .expect("meta table");
    for statement in sentinel::store::schema::migrations()
        .into_iter()
        .find(|(version, _)| *version == 1)
        .expect("v1 exists")
        .1
    {
        db.execute(statement, vec![]).await.expect("apply v1");
    }
    db.execute(
        "INSERT INTO sentinel_meta (key, value) VALUES (?, '1')",
        vec![json!(sentinel::store::schema::SCHEMA_VERSION_KEY)],
    )
    .await
    .expect("stamp v1");

    let version = store.migrate().await.expect("upgrade");
    assert_eq!(version, sentinel::store::schema::SCHEMA_VERSION);
    db.query("SELECT id FROM sentinel_transitions LIMIT 1", vec![])
        .await
        .expect("v2 added the transitions table");

    // And running it again is a no-op rather than a second attempt.
    assert_eq!(
        store.migrate().await.expect("re-run"),
        sentinel::store::schema::SCHEMA_VERSION
    );
}

#[tokio::test]
async fn the_sweeper_cannot_hand_the_same_parked_log_over_twice_a_second() {
    let store = store().await;
    store
        .insert_pending_log(&sentinel::store::PendingLogWrite {
            dedupe_key: "log:-:-:1:abc".into(),
            at_ms: NOW,
            trace_id: None,
            span_id: None,
            session_id: None,
            worker_version: None,
            message: "Function not found".into(),
            evidence: None,
            join_deadline_ms: NOW,
            session_unknown: true,
        })
        .await
        .expect("park the log");

    let first = store
        .due_pending_logs(NOW + 1, 100)
        .await
        .expect("the wait has expired");
    assert_eq!(first.len(), 1, "the row is due");

    // The span never came and the promotion did not clear it. Half a second
    // later the sweeper must not queue it again.
    let second = store
        .due_pending_logs(NOW + 501, 100)
        .await
        .expect("read again");
    assert!(
        second.is_empty(),
        "a row already handed to the queue was handed over again"
    );

    // It does come back, once the retry window has passed.
    let later = store
        .due_pending_logs(NOW + 60_000, 100)
        .await
        .expect("read after the backoff");
    assert_eq!(later.len(), 1, "a stuck row is retried, not abandoned");
}
