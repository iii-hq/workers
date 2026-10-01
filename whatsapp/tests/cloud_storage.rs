use tempfile::TempDir;
use whatsapp::{
    config::{CloudConfig, Config},
    inbox::{ExternalReceipt, Inbox, MessageView},
    sessions::SessionManager,
};

fn message(id: &str, outgoing: bool) -> MessageView {
    MessageView {
        message_id: id.into(),
        chat_id: "cloud:+15551234567".into(),
        sender_id: if outgoing { "me" } else { "+15551234567" }.into(),
        sender_name: (!outgoing).then(|| "Customer".into()),
        from_me: outgoing,
        timestamp_ms: 1_000,
        kind: "text".into(),
        text: "A retained message".into(),
        source: if outgoing { "sent" } else { "live" }.into(),
        status: outgoing.then(|| "accepted".into()),
    }
}

fn receipt(id: &str, status: &str) -> ExternalReceipt {
    ExternalReceipt {
        message_id: id.into(),
        chat_id: "cloud:+15551234567".into(),
        status: status.into(),
    }
}

#[test]
fn cloud_webhook_replay_survives_restart_without_duplicate_messages() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 10).unwrap();
    inbox
        .record_external(
            &[message("incoming", false), message("outgoing", true)],
            &[],
        )
        .unwrap();
    drop(inbox);
    let reopened = Inbox::open(dir.path(), 10).unwrap();
    reopened
        .record_external(
            &[message("incoming", false)],
            &[receipt("outgoing", "delivered")],
        )
        .unwrap();
    let page = reopened
        .list_messages("cloud:+15551234567", None, 10)
        .unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(
        page.messages
            .iter()
            .find(|m| m.from_me)
            .unwrap()
            .status
            .as_deref(),
        Some("delivered")
    );
    assert!(
        reopened
            .list_messages("15551234567@s.whatsapp.net", None, 10)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn receipts_before_send_response_are_retained_and_never_regress() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 10).unwrap();
    inbox
        .record_external(&[], &[receipt("outgoing", "read")])
        .unwrap();
    drop(inbox);
    let inbox = Inbox::open(dir.path(), 10).unwrap();
    inbox
        .record_external(&[message("outgoing", true)], &[])
        .unwrap();
    inbox
        .record_external(
            &[],
            &[
                receipt("outgoing", "sent"),
                receipt("outgoing", "failed"),
                receipt("outgoing", "delivered"),
            ],
        )
        .unwrap();
    assert_eq!(
        inbox
            .list_messages("cloud:+15551234567", None, 10)
            .unwrap()
            .messages[0]
            .status
            .as_deref(),
        Some("read")
    );
    inbox.clear().unwrap();
    inbox
        .record_external(&[message("outgoing", true)], &[])
        .unwrap();
    assert_eq!(
        inbox
            .list_messages("cloud:+15551234567", None, 10)
            .unwrap()
            .messages[0]
            .status
            .as_deref(),
        Some("accepted")
    );
}

#[test]
fn invalid_cloud_batch_cannot_partially_commit_and_retention_applies() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 2).unwrap();
    let mut bad = message("bad", false);
    bad.chat_id = "../escape".into();
    assert!(
        inbox
            .record_external(&[message("good", false), bad], &[])
            .is_err()
    );
    assert!(inbox.list_chats(None, None, 10).unwrap().chats.is_empty());
    inbox
        .record_external(
            &[
                message("one", false),
                message("two", false),
                message("three", false),
            ],
            &[],
        )
        .unwrap();
    assert_eq!(
        inbox
            .list_messages("cloud:+15551234567", None, 10)
            .unwrap()
            .messages
            .len(),
        2
    );
    inbox.set_limit(1).unwrap();
    assert_eq!(
        inbox
            .list_messages("cloud:+15551234567", None, 10)
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[tokio::test]
async fn cloud_storage_is_not_discovered_as_a_native_session() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        data_dir: dir.path().join("sessions"),
        ..Config::default()
    };
    let manager = SessionManager::open(config.clone()).unwrap();
    std::fs::create_dir(config.data_dir.join(".cloud")).unwrap();
    std::fs::create_dir(config.data_dir.join(".cloud/12345")).unwrap();
    drop(manager);
    let reopened = SessionManager::open(config).unwrap();
    assert!(reopened.list().await.is_empty());
}

#[test]
fn cloud_configuration_defaults_are_off_and_debug_never_contains_credentials() {
    assert!(!Config::default().cloud.enabled);
    let config = Config {
        cloud: CloudConfig {
            enabled: true,
            business_account_id: "12345".into(),
            phone_number_id: "67890".into(),
            access_token: "private-access-token".into(),
            app_secret: "private-app-secret".into(),
            verify_token: "private-verify-token".into(),
            ..CloudConfig::default()
        },
        ..Config::default()
    };
    config.validate().unwrap();
    let debug = format!("{config:?}");
    for secret in [
        &config.cloud.access_token,
        &config.cloud.app_secret,
        &config.cloud.verify_token,
    ] {
        assert!(!debug.contains(secret));
    }
    for field in ["access_token", "app_secret", "verify_token"] {
        assert_eq!(
            Config::json_schema()["definitions"]["CloudConfig"]["properties"][field]["writeOnly"],
            true
        );
    }
    let mut invalid = config.clone();
    invalid.cloud.phone_number_id = "../../other".into();
    assert!(invalid.validate().is_err());
    invalid = config;
    invalid.cloud.access_token.clear();
    assert!(invalid.validate().is_err());
}
