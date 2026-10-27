#[path = "support/session_bus.rs"]
mod session_bus;

use provider_openai_codex::credential_store::{SecretsSessionStore, SessionRecord, SessionStore};
use provider_openai_codex::oauth::Credential;
use serde_json::{json, Value};
use session_bus::{Failure, SessionBus, RESOLVE, SECRET_CAS, STATE_CAS};

fn session() -> SessionRecord {
    SessionRecord::connected(Credential {
        access_token: "synthetic-access".into(),
        refresh_token: Some("synthetic-refresh".into()),
        id_token: Some("synthetic-id".into()),
        account_id: "synthetic-account".into(),
        expires_at: 4_102_444_800,
    })
}

fn envelope(record: Option<&SessionRecord>, complete: bool) -> String {
    json!({"version":1,"session":record,"migration_complete":complete}).to_string()
}

#[tokio::test]
async fn legacy_state_worker_must_acknowledge_durable_cleanup() {
    let bus = SessionBus::start(serde_json::to_value(session()).unwrap(), None).await;
    bus.state.lock().unwrap().omit_durable_ack = true;
    assert!(SecretsSessionStore::new(bus.client.clone())
        .load()
        .await
        .is_err());
    let state = bus.state.lock().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(state.secret.as_ref().unwrap()).unwrap()
            ["migration_complete"],
        false
    );
}

#[tokio::test]
async fn migrates_session_and_clears_state() {
    let record = session();
    let bus = SessionBus::start(serde_json::to_value(&record).unwrap(), None).await;
    let store = SecretsSessionStore::new(bus.client.clone());
    let loaded = store.load().await.unwrap().unwrap();
    assert!(serde_json::to_value(loaded).unwrap() == serde_json::to_value(&record).unwrap());
    let state = bus.state.lock().unwrap();
    assert!(state.legacy.is_null() && state.flushed);
    let stored: Value = serde_json::from_str(state.secret.as_ref().unwrap()).unwrap();
    assert_eq!(stored["migration_complete"], true);
    let cleanup = state.calls.iter().position(|s| s == STATE_CAS).unwrap();
    let creation = state.calls.iter().position(|s| s == SECRET_CAS).unwrap();
    assert!(state.calls[creation + 1..cleanup]
        .iter()
        .any(|call| call == RESOLVE));
}

#[tokio::test]
async fn failed_vault_readback_preserves_state_until_migration_can_resume() {
    let record = session();
    let original = serde_json::to_value(&record).unwrap();
    let bus = SessionBus::start(original.clone(), None).await;
    bus.state.lock().unwrap().failure = Some(Failure {
        function: RESOLVE,
        after_write: false,
        // The first resolve finds no secret; fail the read after its creation.
        skip: 1,
    });
    let store = SecretsSessionStore::new(bus.client.clone());
    assert_eq!(
        store.load().await.err().unwrap().code,
        "storage_unavailable"
    );
    {
        let state = bus.state.lock().unwrap();
        assert!(
            state.legacy == original,
            "readback failure changed the original session"
        );
        assert!(!state.flushed);
        assert!(!state.secret_read_back);
        assert!(!state.calls.iter().any(|call| call == STATE_CAS));
        assert_eq!(
            state
                .calls
                .iter()
                .filter(|call| *call == SECRET_CAS)
                .count(),
            1
        );
        let stored: Value = serde_json::from_str(state.secret.as_ref().unwrap()).unwrap();
        assert!(stored["session"] == original);
        assert_eq!(stored["migration_complete"], false);
    }

    // A fresh process resumes from the copied secret, confirms readability,
    // and only then clears the original and marks the migration complete.
    let restarted = SecretsSessionStore::new(bus.client.clone());
    let loaded = restarted.load().await.unwrap().unwrap();
    assert!(serde_json::to_value(loaded).unwrap() == original);
    let state = bus.state.lock().unwrap();
    assert!(state.legacy.is_null() && state.flushed && state.secret_read_back);
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|call| *call == SECRET_CAS)
            .count(),
        2
    );
    assert_eq!(
        state.calls.iter().filter(|call| *call == STATE_CAS).count(),
        1
    );
    let stored: Value = serde_json::from_str(state.secret.as_ref().unwrap()).unwrap();
    assert_eq!(stored["migration_complete"], true);
}

#[tokio::test]
async fn migration_resumes_after_write_or_cleanup_response_is_lost() {
    for failure in [
        Failure {
            function: SECRET_CAS,
            after_write: true,
            skip: 0,
        },
        Failure {
            function: STATE_CAS,
            after_write: false,
            skip: 0,
        },
        Failure {
            function: STATE_CAS,
            after_write: true,
            skip: 0,
        },
        Failure {
            function: SECRET_CAS,
            after_write: true,
            skip: 1,
        },
    ] {
        let record = session();
        let bus = SessionBus::start(serde_json::to_value(&record).unwrap(), None).await;
        bus.state.lock().unwrap().failure = Some(failure);
        assert!(SecretsSessionStore::new(bus.client.clone())
            .load()
            .await
            .is_err());
        let restarted = SecretsSessionStore::new(bus.client.clone());
        let loaded = restarted.load().await.unwrap().unwrap();
        assert_eq!(loaded.revision, record.revision);
        let state = bus.state.lock().unwrap();
        assert!(state.legacy.is_null() && state.flushed);
        assert_eq!(
            serde_json::from_str::<Value>(state.secret.as_ref().unwrap()).unwrap()
                ["migration_complete"],
            true
        );
    }
}

#[tokio::test]
async fn cleanup_failure_and_flush_retry_keep_migration_pending() {
    let record = session();
    let bus = SessionBus::start(serde_json::to_value(&record).unwrap(), None).await;
    bus.state.lock().unwrap().failure = Some(Failure {
        function: STATE_CAS,
        after_write: true,
        skip: 0,
    });
    let store = SecretsSessionStore::new(bus.client.clone());
    assert!(store.load().await.is_err());
    {
        let state = bus.state.lock().unwrap();
        assert!(state.legacy.is_null() && !state.flushed);
        assert_eq!(
            serde_json::from_str::<Value>(state.secret.as_ref().unwrap()).unwrap()
                ["migration_complete"],
            false
        );
    }
    assert!(store.load().await.unwrap().is_some());
    assert!(bus.state.lock().unwrap().flushed);
}

#[tokio::test]
async fn existing_vault_logout_wins_and_does_not_read_state() {
    let logout = SessionRecord::logged_out();
    let bus = SessionBus::start(
        serde_json::to_value(session()).unwrap(),
        Some(envelope(Some(&logout), true)),
    )
    .await;
    let store = SecretsSessionStore::new(bus.client.clone());
    assert!(store.load().await.unwrap().unwrap().credential.is_none());
    assert!(bus.state.lock().unwrap().calls.iter().all(|s| s == RESOLVE));
}

#[tokio::test]
async fn changed_legacy_session_is_not_deleted() {
    let bus = SessionBus::start(serde_json::to_value(session()).unwrap(), None).await;
    bus.state.lock().unwrap().change_before_cleanup = true;
    assert!(SecretsSessionStore::new(bus.client.clone())
        .load()
        .await
        .is_err());
    assert_eq!(
        bus.state.lock().unwrap().legacy["revision"],
        "concurrent-revision"
    );
}

#[tokio::test]
async fn missing_managed_session_preserves_legacy_resolution_and_logout_is_distinct() {
    let bus = SessionBus::start(Value::Null, None).await;
    let store = SecretsSessionStore::new(bus.client.clone());
    assert!(store.load().await.unwrap().is_none());
    let logout = SessionRecord::logged_out();
    assert!(store.compare_and_set(None, &logout).await.unwrap());
    assert!(store.load().await.unwrap().unwrap().credential.is_none());
}

#[tokio::test]
async fn storage_errors_and_invalid_envelopes_are_not_absence() {
    for value in [
        "not-json",
        "{}",
        r#"{"version":1,"migration_complete":true}"#,
        r#"{"version":2,"session":null,"migration_complete":true}"#,
    ] {
        let bus = SessionBus::start(Value::Null, Some(value.into())).await;
        assert_eq!(
            SecretsSessionStore::new(bus.client.clone())
                .load()
                .await
                .err()
                .unwrap()
                .code,
            "storage_unavailable"
        );
    }
    for code in ["SECRET_FORBIDDEN", "KEY_MISMATCH", "STORAGE_UNAVAILABLE"] {
        let bus = SessionBus::start(Value::Null, None).await;
        bus.state.lock().unwrap().resolve_error = Some(code);
        assert_eq!(
            SecretsSessionStore::new(bus.client.clone())
                .load()
                .await
                .err()
                .unwrap()
                .code,
            "storage_unavailable"
        );
        assert_eq!(bus.state.lock().unwrap().calls, [RESOLVE]);
    }
}

#[tokio::test]
async fn compare_and_set_preserves_newer_session_and_uses_original_json() {
    let record = session();
    let original = serde_json::to_string_pretty(
        &serde_json::from_str::<Value>(&envelope(Some(&record), true)).unwrap(),
    )
    .unwrap();
    let bus = SessionBus::start(Value::Null, Some(original)).await;
    let store = SecretsSessionStore::new(bus.client.clone());
    let expected = store.load().await.unwrap().unwrap();
    let logout = SessionRecord::logged_out();
    assert!(store
        .compare_and_set(Some(&expected), &logout)
        .await
        .unwrap());
    assert!(!store
        .compare_and_set(Some(&expected), &session())
        .await
        .unwrap());
    assert!(store.load().await.unwrap().unwrap().credential.is_none());
}

#[tokio::test]
async fn migrates_logout_and_expired_session_without_reactivating_them() {
    let mut expired = session();
    expired.needs_login = true;
    for record in [SessionRecord::logged_out(), expired] {
        let bus = SessionBus::start(serde_json::to_value(&record).unwrap(), None).await;
        let loaded = SecretsSessionStore::new(bus.client.clone())
            .load()
            .await
            .unwrap()
            .unwrap();
        assert!(serde_json::to_value(loaded).unwrap() == serde_json::to_value(record).unwrap());
        assert!(bus.state.lock().unwrap().legacy.is_null());
    }
}
