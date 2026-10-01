use serde_json::json;
use tempfile::TempDir;
use whatsapp::config::Config;
use whatsapp::sessions::{SessionManager, SessionStatus};

fn config(directory: &TempDir) -> Config {
    Config {
        data_dir: directory.path().join("sessions"),
        max_sessions: 2,
        max_events: 3,
        ..Config::default()
    }
}

#[tokio::test]
async fn creation_and_reopen_remain_offline_and_preserve_metadata() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    let first = manager.create("personal").await.unwrap();
    assert_eq!(first.status, SessionStatus::Stopped);
    assert!(manager.client("personal").await.is_err());
    assert!(manager.qr("personal").await.unwrap().is_none());
    assert!(
        !directory
            .path()
            .join("sessions/personal/credentials.db")
            .exists()
    );
    drop(manager);
    let reopened = SessionManager::open(config(&directory)).unwrap();
    assert_eq!(reopened.get("personal").await.unwrap(), first);
    assert_eq!(reopened.list().await.len(), 1);
    assert!(reopened.client("personal").await.is_err());
}

#[tokio::test]
async fn unsafe_identifiers_never_create_paths() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    for id in [
        "",
        ".",
        "..",
        "../outside",
        "a/b",
        "a\\b",
        "/tmp/x",
        "a b",
        "á",
        "a.json",
    ] {
        assert!(manager.create(id).await.is_err(), "accepted {id:?}");
    }
    assert!(manager.create(&"x".repeat(65)).await.is_err());
    assert!(manager.list().await.is_empty());
    assert!(manager.create("work_2-main").await.is_ok());
}

#[tokio::test]
async fn concurrent_creation_enforces_uniqueness_and_capacity() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    let (left, right) = tokio::join!(manager.create("same"), manager.create("same"));
    assert_ne!(left.is_ok(), right.is_ok());
    let (left, right) = tokio::join!(manager.create("second"), manager.create("third"));
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(manager.list().await.len(), 2);
}

#[tokio::test]
async fn live_session_limit_preserves_existing_accounts_and_survives_restart() {
    let directory = TempDir::new().unwrap();
    let initial = config(&directory);
    let manager = SessionManager::open(initial.clone()).unwrap();
    manager.create("first").await.unwrap();
    manager.create("second").await.unwrap();
    let lower = Config {
        max_sessions: 1,
        ..initial.clone()
    };
    assert!(manager.apply_config(lower.clone()).unwrap().is_empty());
    assert!(manager.create("third").await.is_err());
    assert_eq!(manager.list().await.len(), 2);
    assert_eq!(
        manager.get("first").await.unwrap().status,
        SessionStatus::Stopped
    );
    drop(manager);

    let manager = SessionManager::open(lower).unwrap();
    assert_eq!(manager.list().await.len(), 2);
    assert!(manager.create("third").await.is_err());
    manager
        .apply_config(Config {
            max_sessions: 3,
            ..initial
        })
        .unwrap();
    assert!(manager.create("third").await.is_ok());
}

#[tokio::test]
async fn structural_reload_keeps_existing_and_new_sessions_on_live_storage_and_event_capacity() {
    let directory = TempDir::new().unwrap();
    let initial = config(&directory);
    let manager = SessionManager::open(initial.clone()).unwrap();
    manager.create("before").await.unwrap();
    let next_storage = directory.path().join("next-start");
    assert_eq!(
        manager
            .apply_config(Config {
                data_dir: next_storage.clone(),
                max_events: 1,
                ..initial.clone()
            })
            .unwrap(),
        ["data_dir", "max_events"]
    );
    manager.create("after").await.unwrap();
    for id in ["before", "after"] {
        assert!(initial.data_dir.join(id).join("metadata.json").is_file());
        for number in 0..4 {
            manager
                .publish(id, "message.sent", json!({"number": number}))
                .await
                .unwrap();
        }
        let page = manager.events(id, 0, 10).await.unwrap();
        assert_eq!(page.events.len(), 3);
        assert!(page.truncated);
        assert_eq!(page.events[0].data["number"], 1);
    }
    assert!(!next_storage.exists());
}

#[tokio::test]
async fn deleting_one_session_preserves_other_credentials_and_is_not_resurrected() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    manager.create("first").await.unwrap();
    manager.create("second").await.unwrap();
    let retained = directory.path().join("sessions/second/credentials.db");
    std::fs::write(&retained, b"credentials belonging to second").unwrap();
    manager.stop("first").await.unwrap();
    manager.stop("first").await.unwrap();
    manager.delete("first").await.unwrap();
    assert!(manager.get("first").await.is_err());
    assert_eq!(
        std::fs::read(&retained).unwrap(),
        b"credentials belonging to second"
    );
    drop(manager);
    let reopened = SessionManager::open(config(&directory)).unwrap();
    assert_eq!(reopened.list().await.len(), 1);
    assert_eq!(reopened.list().await[0].session_id, "second");
}

#[tokio::test]
async fn bounded_events_have_monotonic_cursors_and_report_dropped_history() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    manager.create("work").await.unwrap();
    let mut subscriber = manager.subscribe();
    let mut latest = 0;
    for number in 0..5 {
        let event = manager
            .publish("work", "message.sent", json!({ "number": number }))
            .await
            .unwrap();
        assert!(event.sequence > latest);
        latest = event.sequence;
    }
    let page = manager.events("work", 0, 10).await.unwrap();
    assert_eq!(page.events.len(), 3);
    assert!(page.truncated);
    assert_eq!(page.events[0].data, json!({"number": 2}));
    assert_eq!(page.next_sequence, latest);
    let next = manager
        .events("work", page.events[0].sequence, 1)
        .await
        .unwrap();
    assert_eq!(next.events.len(), 1);
    assert_eq!(next.events[0].data, json!({"number": 3}));
    assert!(
        manager
            .events("work", latest, 10)
            .await
            .unwrap()
            .events
            .is_empty()
    );
    // Broadcast overflow is observable, rather than creating an unbounded queue.
    assert!(matches!(
        subscriber.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
    ));
}

#[tokio::test]
async fn event_cursor_never_goes_backwards_after_reopening() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    manager.create("work").await.unwrap();
    let previous = manager
        .publish("work", "message.sent", json!({}))
        .await
        .unwrap();
    drop(manager);
    let manager = SessionManager::open(config(&directory)).unwrap();
    let current = manager
        .publish("work", "message.sent", json!({}))
        .await
        .unwrap();
    assert!(current.sequence > previous.sequence);
    let page = manager.events("work", previous.sequence, 10).await.unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.truncated);
}

#[tokio::test]
async fn oversized_event_is_rejected_without_retaining_or_broadcasting_it() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    manager.create("work").await.unwrap();
    let mut subscriber = manager.subscribe();
    assert!(
        manager
            .publish("work", "message.sent", json!({"text": "x".repeat(65_537)}))
            .await
            .is_err()
    );
    assert!(
        manager
            .events("work", 0, 10)
            .await
            .unwrap()
            .events
            .is_empty()
    );
    assert!(subscriber.try_recv().is_err());
}

#[tokio::test]
async fn shutdown_leaves_persistence_and_rejects_new_lifecycle_work() {
    let directory = TempDir::new().unwrap();
    let manager = SessionManager::open(config(&directory)).unwrap();
    manager.create("work").await.unwrap();
    manager.shutdown().await;
    assert_eq!(
        manager.get("work").await.unwrap().status,
        SessionStatus::Stopped
    );
    assert!(manager.start("work").await.is_err());
    assert!(manager.create("late").await.is_err());
    assert!(
        SessionManager::open(config(&directory))
            .unwrap()
            .get("work")
            .await
            .is_ok()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_session_directory_is_rejected_without_touching_target() {
    let directory = TempDir::new().unwrap();
    let configuration = config(&directory);
    std::fs::create_dir_all(&configuration.data_dir).unwrap();
    let outside = directory.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("metadata.json"), b"do not modify").unwrap();
    std::os::unix::fs::symlink(&outside, configuration.data_dir.join("linked")).unwrap();
    assert!(SessionManager::open(configuration).is_err());
    assert_eq!(
        std::fs::read(outside.join("metadata.json")).unwrap(),
        b"do not modify"
    );
}
