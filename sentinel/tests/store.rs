//! The store's statements, against a real SQLite (or Postgres; see support/db.rs).
//!
//! These are the tests that would have caught the data-model defects the
//! adversarial review found: a pending log needs a row before its group
//! exists, and a group's transition has to survive somebody else writing
//! between the read and the write.

#[path = "support/db.rs"]
mod db;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use db::{SqliteDb, TestDb};
use sentinel::store::schema::SCHEMA_VERSION;
use sentinel::store::PendingLogWrite;
use sentinel::{
    Db, ErrorSourceV1, GroupStatusV1, IgnoreBaselineV1, IgnoreRuleV1, NamedRow, OccurrenceWrite,
    RecordOutcome, SentinelError, Statement, StepResult, Store,
};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

const NOW: i64 = 1_789_000_000_000;

async fn store() -> Store<TestDb> {
    let store = Store::new(db::test_db().await);
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

async fn column(store: &Store<TestDb>, sql: &str, params: Vec<Value>) -> Value {
    let rows = store.db().query(sql, params).await.expect("query");
    rows.first()
        .and_then(|row| row.values().next().cloned())
        .map(db::plain)
        .unwrap_or(Value::Null)
}

async fn set_group(store: &Store<TestDb>, group_id: &str, sql: &str, params: Vec<Value>) {
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
    // Postgres, through the `database` worker, names only the SQLSTATE.
    let error = error.to_string().to_lowercase();
    assert!(
        error.contains("check") || error.contains("23514"),
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
    let store = Store::new(db::test_db().await);
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

/// What the parked-log sweep reads, and a way to hold its deadline read open so
/// a log can park between the read and the wait.
struct WatchedDb {
    inner: TestDb,
    deadline_reads: AtomicU64,
    due_reads: AtomicU64,
    hold_deadline_read: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
    /// The next transaction commits and then fails, as a call whose reply
    /// timed out does.
    lose_next_reply: AtomicBool,
}

#[async_trait]
impl Db for WatchedDb {
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<NamedRow>, SentinelError> {
        if sql.contains("MIN(join_deadline_ms)") {
            self.deadline_reads.fetch_add(1, Ordering::Relaxed);
            let rows = self.inner.query(sql, params).await;
            if let Some((read, resume)) = self.hold_deadline_read.lock().await.take() {
                read.send(()).expect("the test is waiting");
                resume.await.expect("the test releases the read");
            }
            return rows;
        }
        if sql.contains("join_deadline_ms <=") {
            self.due_reads.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.query(sql, params).await
    }

    async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, SentinelError> {
        self.inner.execute(sql, params).await
    }

    async fn transaction(
        &self,
        statements: &[Statement],
    ) -> Result<Vec<StepResult>, SentinelError> {
        let committed = self.inner.transaction(statements).await?;
        if self.lose_next_reply.swap(false, Ordering::Relaxed) {
            return Err(SentinelError::dependency("invocation timed out"));
        }
        Ok(committed)
    }
}

async fn watched_store() -> Arc<Store<WatchedDb>> {
    watching(db::test_db().await).await
}

/// For a paused clock, which runs ahead while a socket waits: never Postgres.
async fn watched_sqlite_store() -> Arc<Store<WatchedDb>> {
    watching(TestDb::Sqlite(SqliteDb::in_memory())).await
}

async fn watching(inner: TestDb) -> Arc<Store<WatchedDb>> {
    let store = Store::new(WatchedDb {
        inner,
        deadline_reads: AtomicU64::new(0),
        due_reads: AtomicU64::new(0),
        hold_deadline_read: Mutex::new(None),
        lose_next_reply: AtomicBool::new(false),
    });
    store.migrate().await.expect("migrate a fresh database");
    Arc::new(store)
}

/// A row this process rang no bell for, as another sentinel on the same store
/// or a previous run leaves it.
async fn parked_behind_the_sweeps_back(store: &Store<WatchedDb>, id: &str) {
    store
        .db()
        .transaction(&[Statement::new(
            "INSERT INTO sentinel_occurrences (id, group_id, dedupe_key, source, at_ms, message, \
             pending_join, join_deadline_ms) VALUES (?, NULL, ?, 'log', ?, ?, 1, ?)",
            vec![
                json!(id),
                json!(format!("log:-:-:1:{id}")),
                json!(NOW),
                json!("Function not found"),
                json!(NOW),
            ],
        )])
        .await
        .expect("a row nobody announced");
}

fn parked_log(dedupe_key: &str, join_deadline_ms: i64) -> PendingLogWrite {
    PendingLogWrite {
        dedupe_key: dedupe_key.into(),
        at_ms: join_deadline_ms,
        trace_id: None,
        span_id: None,
        session_id: None,
        worker_version: None,
        message: "Function not found".into(),
        evidence: None,
        join_deadline_ms,
        session_unknown: true,
    }
}

const NOT_WITHIN: Duration = Duration::from_millis(150);
const WITHIN: Duration = Duration::from_secs(5);
/// Past the longest the sweep may sleep, on a paused clock.
const WITHIN_THE_RESCAN: Duration = Duration::from_secs(45);

/// Each of these reads is three root traces in the engine's ring, so a sweep
/// polling an empty table is what fills it.
#[tokio::test(start_paused = true)]
async fn a_sweep_with_nothing_parked_reads_only_twice_a_minute_and_never_for_due_rows() {
    let store = watched_sqlite_store().await;

    timeout(Duration::from_secs(295), store.next_due_pending_logs(100))
        .await
        .expect_err("nothing can be due");

    assert_eq!(store.db().deadline_reads.load(Ordering::Relaxed), 10);
    assert_eq!(store.db().due_reads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn parking_a_log_wakes_the_sweep_and_it_claims_the_log_when_the_wait_runs_out() {
    let store = watched_store().await;
    let sweep = tokio::spawn({
        let store = store.clone();
        async move { store.next_due_pending_logs(100).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(store.db().deadline_reads.load(Ordering::Relaxed), 1);

    let id = store
        .insert_pending_log(&parked_log("log:-:-:1:wake", sentinel::ids::now_ms() + 100))
        .await
        .expect("park the log");

    let due = timeout(WITHIN, sweep)
        .await
        .expect("the park wakes the sweep")
        .expect("the sweep did not panic")
        .expect("the read works");
    assert_eq!(due.len(), 1);
    assert_eq!(Some(due[0].id.clone()), id);
    assert_eq!(
        store.db().due_reads.load(Ordering::Relaxed),
        1,
        "no due query before the wait ran out"
    );
    assert_eq!(
        store.db().deadline_reads.load(Ordering::Relaxed),
        3,
        "nothing parked, then a wait to sleep through, then due: nothing in between"
    );
}

#[tokio::test]
async fn a_log_parked_with_an_earlier_deadline_cuts_the_sweeps_sleep_short() {
    let store = watched_store().await;
    let far = sentinel::ids::now_ms() + 3_600_000;
    store
        .insert_pending_log(&parked_log("log:-:-:1:far", far))
        .await
        .expect("park the far log");
    let sweep = tokio::spawn({
        let store = store.clone();
        async move { store.next_due_pending_logs(100).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    store
        .insert_pending_log(&parked_log("log:-:-:1:near", sentinel::ids::now_ms() - 1))
        .await
        .expect("park the near log");

    let due = timeout(WITHIN, sweep)
        .await
        .expect("the sweep does not sleep through the earlier deadline")
        .expect("the sweep did not panic")
        .expect("the read works");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].dedupe_key, "log:-:-:1:near");
}

#[tokio::test]
async fn a_due_row_is_claimed_once_and_does_not_wake_the_sweep_again() {
    let store = watched_store().await;
    store
        .insert_pending_log(&parked_log("log:-:-:1:due", sentinel::ids::now_ms() - 1))
        .await
        .expect("park the log");

    let due = timeout(WITHIN, store.next_due_pending_logs(100))
        .await
        .expect("the row is due")
        .expect("the read works");
    assert_eq!(due.len(), 1);

    timeout(NOT_WITHIN, store.next_due_pending_logs(100))
        .await
        .expect_err("the claim pushed the deadline forward");
    assert_eq!(
        store.db().due_reads.load(Ordering::Relaxed),
        1,
        "the claimed row must not be queued in a loop"
    );
}

#[tokio::test]
async fn a_row_parked_before_the_sweep_starts_is_found() {
    let store = watched_store().await;
    parked_behind_the_sweeps_back(&store, "occ_restart").await;

    let due = timeout(WITHIN, store.next_due_pending_logs(100))
        .await
        .expect("the first pass reads the store")
        .expect("the read works");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "occ_restart");
}

/// The commit went through and only its reply was lost. The queue's retry then
/// dedupes against the row, so neither attempt tells the sweep it is there.
#[tokio::test(start_paused = true)]
async fn a_park_whose_reply_was_lost_is_still_claimed_by_the_sleeping_sweep() {
    let store = watched_sqlite_store().await;
    let sweep = tokio::spawn({
        let store = store.clone();
        async move { store.next_due_pending_logs(100).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let log = parked_log("log:-:-:1:lost-reply", sentinel::ids::now_ms() - 1);
    store.db().lose_next_reply.store(true, Ordering::Relaxed);
    store
        .insert_pending_log(&log)
        .await
        .expect_err("the reply of the commit was lost");
    assert_eq!(
        store.insert_pending_log(&log).await.expect("the retry"),
        None,
        "the retry dedupes against the parked row"
    );

    let due = timeout(WITHIN_THE_RESCAN, sweep)
        .await
        .expect("nothing else will ever promote this row")
        .expect("the sweep did not panic")
        .expect("the read works");
    assert_eq!(due.len(), 1);
}

/// The notification stops at this process, and a store can be shared by the team.
#[tokio::test(start_paused = true)]
async fn a_row_another_sentinel_left_parked_is_claimed_by_this_ones_sleeping_sweep() {
    let store = watched_sqlite_store().await;
    let sweep = tokio::spawn({
        let store = store.clone();
        async move { store.next_due_pending_logs(100).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    parked_behind_the_sweeps_back(&store, "occ_teammate").await;

    let due = timeout(WITHIN_THE_RESCAN, sweep)
        .await
        .expect("its owner may never come back")
        .expect("the sweep did not panic")
        .expect("the read works");
    assert_eq!(due[0].id, "occ_teammate");
}

#[tokio::test(start_paused = true)]
async fn a_far_deadline_does_not_hide_a_row_another_sentinel_left_due() {
    let store = watched_sqlite_store().await;
    store
        .insert_pending_log(&parked_log(
            "log:-:-:1:far",
            sentinel::ids::now_ms() + 3_600_000,
        ))
        .await
        .expect("park the far log");
    let sweep = tokio::spawn({
        let store = store.clone();
        async move { store.next_due_pending_logs(100).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    parked_behind_the_sweeps_back(&store, "occ_teammate").await;

    let due = timeout(WITHIN_THE_RESCAN, sweep)
        .await
        .expect("the sweep looks again before the far deadline")
        .expect("the sweep did not panic")
        .expect("the read works");
    assert_eq!(due[0].id, "occ_teammate");
}

#[tokio::test]
async fn a_log_parked_between_the_read_and_the_wait_is_not_lost() {
    let store = watched_store().await;
    let (read, deadline_read) = oneshot::channel();
    let (release, resume) = oneshot::channel();
    *store.db().hold_deadline_read.lock().await = Some((read, resume));

    let sweep = tokio::spawn({
        let store = store.clone();
        async move { store.next_due_pending_logs(100).await }
    });
    deadline_read.await.expect("the sweep read the deadline");

    // The sweep has its answer, nothing parked, and has not started waiting.
    store
        .insert_pending_log(&parked_log("log:-:-:1:race", sentinel::ids::now_ms() - 1))
        .await
        .expect("park the log");
    release.send(()).expect("the sweep is held");

    let due = timeout(WITHIN, sweep)
        .await
        .expect("the park was not lost")
        .expect("the sweep did not panic")
        .expect("the read works");
    assert_eq!(due.len(), 1);
}
