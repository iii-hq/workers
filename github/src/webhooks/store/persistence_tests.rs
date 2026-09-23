use super::*;
use crate::webhooks::types::*;
use serde_json::json;

fn inbox(delivery: &str) -> Job {
    Job::Inbox(Inbox {
        repo: "owner/repo".into(),
        event: "pull_request".into(),
        delivery: delivery.into(),
        body: json!({"data": "x".repeat(1024)}),
    })
}
fn watch(id: &str) -> Watch {
    Watch {
        spec: WatchSpec {
            watch_id: id.into(),
            repo: "owner/repo".into(),
            number: 1208,
            events: Default::default(),
            stop_on: StopOn::Merged,
            expires_at: chrono::Utc::now() + chrono::Duration::days(1),
        },
        status: WatchState::Completed,
        snapshot: Snapshot {
            merged: true,
            ..Default::default()
        },
        lease_id: None,
        error: None,
        seen: Default::default(),
    }
}
fn hook() -> RepoHook {
    RepoHook {
        endpoint_id: "private-endpoint".into(),
        secret: "preserved-secret".into(),
        hook_id: Some(42),
        url: Some("https://example.test/hooks".into()),
        pending_url: None,
        generation: Some("generation".into()),
        create_started: true,
        cleanup_attempts: 3,
        error: None,
    }
}
fn legacy(path: &Path, text: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE state(id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute("INSERT INTO state VALUES(1,?1)", [text])
        .unwrap();
}
fn prune_at(store: &Store, now: i64) {
    let mut db = store.database.lock().unwrap();
    let mut data = db.data.clone();
    let tx = db.connection.transaction().unwrap();
    sql::prune(&tx, &mut data, now).unwrap();
    tx.commit().unwrap();
    sql::clean(&mut data);
    db.data = data;
}
#[test]
fn legacy_migration_preserves_installation_secrets_inbox_outbox_and_subscriber_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let mut data = Data {
        installation: "existing-installation".into(),
        last_error: Some("offline".into()),
        tunnel_status: "ready".into(),
        ..Default::default()
    };
    let mut w = watch("existing");
    w.status = WatchState::Active;
    w.lease_id = Some("lease".into());
    w.seen.insert("fingerprint".into());
    data.watches.insert("existing".into(), w);
    data.repos.insert("owner/repo".into(), hook());
    let sub = Subscriber {
        id: "sub".into(),
        function_id: "handler".into(),
        filter: EventFilter::default(),
        metadata: Some(json!({"tenant":"test"})),
        namespace: Some("private".into()),
    };
    let event = crate::webhooks::normalize::make_event(
        data.watches.get("existing").unwrap(),
        Category::Pr,
        "closed",
        "pr",
        1,
        true,
    );
    data.jobs.insert(
        "notify:one".into(),
        Job::Notify {
            event: Box::new(event),
            target: Box::new(sub.clone()),
        },
    );
    data.subscribers.insert("sub".into(), sub);
    data.jobs.insert("inbox:42:abc".into(), inbox("abc"));
    data.deliveries.insert("42:abc".into());
    data.publications.insert("inbox:42:abc".into(), (123, 4));
    let expected = serde_json::to_value(&data).unwrap();
    legacy(&path, &expected.to_string());
    let store = Store::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(store.read().unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::to_value(Store::inspect(&path).unwrap()).unwrap(),
        expected
    );
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.read().unwrap()).unwrap(),
        expected
    );
    let db = reopened.database.lock().unwrap();
    assert_eq!(
        db.connection
            .query_row("SELECT COUNT(*) FROM state", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        db.connection
            .execute("INSERT OR IGNORE INTO state VALUES(1,'{}')", [])
            .is_err(),
        "old binaries must fail closed"
    );
}
#[test]
fn corrupt_legacy_migration_rolls_back_schema_and_keeps_original_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    legacy(&path, "{invalid");
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT value FROM state", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "{invalid"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='metadata'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn accepting_one_delivery_only_inserts_two_rows_and_shares_existing_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.sqlite3")).unwrap();
    store
        .change(|data| {
            for n in 0..1000 {
                data.jobs.insert(format!("old:{n}"), inbox(&n.to_string()));
            }
            Ok(())
        })
        .unwrap();
    let before = store.read().unwrap();
    let changes = store.database.lock().unwrap().connection.total_changes();
    store
        .change(|data| {
            data.deliveries.insert("42:new".into());
            data.jobs.insert("inbox:42:new".into(), inbox("new"));
            Ok(())
        })
        .unwrap();
    let after = store.read().unwrap();
    assert_eq!(
        store.database.lock().unwrap().connection.total_changes() - changes,
        2
    );
    assert!(std::ptr::eq(
        before.jobs.get("old:0").unwrap(),
        after.jobs.get("old:0").unwrap()
    ));
    assert_eq!((before.jobs.len(), after.jobs.len()), (1000, 1001));
}
#[test]
fn closure_error_and_sql_error_leave_both_cache_and_database_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let store = Store::open(&path).unwrap();
    let expected = serde_json::to_value(store.read().unwrap()).unwrap();
    assert!(store
        .change::<()>(|d| {
            d.jobs.insert("new".into(), inbox("new"));
            Err(Failure::Capacity)
        })
        .is_err());
    store.database.lock().unwrap().connection.execute_batch("CREATE TRIGGER fail_jobs BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(store
        .change(|d| {
            d.last_error = Some("should rollback".into());
            d.deliveries.insert("42:new".into());
            d.jobs.insert("new".into(), inbox("new"));
            Ok(())
        })
        .is_err());
    assert_eq!(
        serde_json::to_value(store.read().unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::to_value(Store::inspect(&path).unwrap()).unwrap(),
        expected
    );
}
#[test]
fn retention_never_removes_pending_jobs_dedupe_or_cleanup_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let store = Store::open(&path).unwrap();
    store
        .change(|d| {
            d.jobs.insert("inbox:42:pending".into(), inbox("pending"));
            d.deliveries.insert("42:pending".into());
            d.deliveries.insert("42:done".into());
            let mut w = watch("blocked");
            w.status = WatchState::CleanupPending;
            w.lease_id = Some("lease".into());
            d.watches.insert("blocked".into(), w);
            d.repos.insert("owner/repo".into(), hook());
            Ok(())
        })
        .unwrap();
    prune_at(
        &store,
        chrono::Utc::now().timestamp() + sql::RETENTION_SECONDS + 120,
    );
    let data = Store::inspect(&path).unwrap();
    assert!(data.jobs.contains_key("inbox:42:pending"));
    assert!(data.deliveries.contains("42:pending"));
    assert!(!data.deliveries.contains("42:done"));
    assert_eq!(data.repos["owner/repo"].secret, "preserved-secret");
    assert_eq!(data.watches["blocked"].lease_id.as_deref(), Some("lease"));
}
#[test]
fn completed_snapshots_survive_cleanup_temporarily_but_not_forever() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.sqlite3")).unwrap();
    store
        .change(|d| {
            d.watches.insert("done".into(), watch("done"));
            Ok(())
        })
        .unwrap();
    assert!(store.read().unwrap().watches["done"].snapshot.merged);
    prune_at(
        &store,
        chrono::Utc::now().timestamp() + sql::RETENTION_SECONDS + 120,
    );
    assert!(!store.read().unwrap().watches.contains_key("done"));
}
#[test]
fn pending_notification_pins_terminal_snapshot_until_acknowledged() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.sqlite3")).unwrap();
    store
        .change(|d| {
            let w = watch("done");
            let event =
                crate::webhooks::normalize::make_event(&w, Category::Pr, "closed", "pr", 1, true);
            let target = Subscriber {
                id: "sub".into(),
                function_id: "handler".into(),
                filter: EventFilter::default(),
                metadata: None,
                namespace: None,
            };
            d.jobs.insert(
                "notify:pending".into(),
                Job::Notify {
                    event: Box::new(event),
                    target: Box::new(target),
                },
            );
            d.watches.insert("done".into(), w);
            Ok(())
        })
        .unwrap();
    let future = chrono::Utc::now().timestamp() + 2 * sql::RETENTION_SECONDS;
    prune_at(&store, future);
    assert!(store.read().unwrap().watches.contains_key("done"));
    store
        .change(|d| {
            d.jobs.remove("notify:pending");
            Ok(())
        })
        .unwrap();
    prune_at(&store, future);
    assert!(store.read().unwrap().watches.contains_key("done"));
    prune_at(&store, future + sql::RETENTION_SECONDS + 1);
    assert!(!store.read().unwrap().watches.contains_key("done"));
}
#[test]
fn seen_and_terminal_counts_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let store = Store::open(&path).unwrap();
    store
        .change(|d| {
            let mut w = watch("active");
            w.status = WatchState::Active;
            for n in 0..sql::SEEN_LIMIT + 10 {
                w.seen.insert(format!("fingerprint:{n:05}"));
            }
            d.watches.insert("active".into(), w);
            for n in 0..sql::TERMINAL_LIMIT + 10 {
                d.watches.insert(
                    format!("terminal:{n:05}"),
                    watch(&format!("terminal:{n:05}")),
                );
            }
            Ok(())
        })
        .unwrap();
    prune_at(
        &store,
        chrono::Utc::now().timestamp() + sql::MIN_TERMINAL_SECONDS + 1,
    );
    let data = Store::inspect(&path).unwrap();
    assert_eq!(data.watches["active"].seen.len(), sql::SEEN_LIMIT);
    assert_eq!(data.watches.len(), sql::TERMINAL_LIMIT + 1);
}
#[test]
fn mutable_iteration_tracks_writes_and_failed_mutation_keeps_old_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.sqlite3")).unwrap();
    store
        .change(|d| {
            d.watches.insert("w".into(), watch("w"));
            Ok(())
        })
        .unwrap();
    let old = store.read().unwrap();
    store
        .change(|d| {
            for (_, w) in &mut d.watches {
                w.error = Some("updated".into());
            }
            Ok(())
        })
        .unwrap();
    assert!(old.watches["w"].error.is_none());
    drop(store);
    assert_eq!(
        Store::open(&dir.path().join("store.sqlite3"))
            .unwrap()
            .read()
            .unwrap()
            .watches["w"]
            .error
            .as_deref(),
        Some("updated")
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn slow_storage_does_not_starve_runtime_timer() {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(&dir.path().join("store.sqlite3")).unwrap());
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        store.change(|_| {
            started_tx.send(()).unwrap();
            finish_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok(())
        })
    });
    started_rx.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    finish_tx.send(()).unwrap();
    task.await.unwrap().unwrap();
}
