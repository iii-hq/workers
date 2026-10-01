use serde_json::{Value, json};
use tempfile::TempDir;
use whatsapp::inbox::Inbox;

fn event(chat: &str, sender: &str, id: &str, timestamp: i64, body: Value) -> Value {
    json!({"kind":"message.received", "data": {
        "info": {"id":id,"timestamp_ms":timestamp,"push_name":"Alice",
            "source":{"chat":chat,"sender":sender,"is_from_me":false}},
        "message":body
    }})
}

#[test]
fn archive_survives_reopen_and_deduplicates_by_chat_sender_and_message() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 100).unwrap();
    let row = event(
        "123@g.us",
        "10@s.whatsapp.net",
        "same",
        1000,
        json!({"conversation":"hello"}),
    );
    inbox
        .import_event_page(&json!({"events":[row.clone(),row]}))
        .unwrap();
    inbox
        .import_event_page(&json!([event(
            "123@g.us",
            "20@s.whatsapp.net",
            "same",
            2000,
            json!({"conversation":"second sender"})
        )]))
        .unwrap();
    drop(inbox);
    let inbox = Inbox::open(dir.path(), 100).unwrap();
    let page = inbox.list_messages("123@g.us", None, 50).unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].text, "second sender");
    assert!(!dir.path().join("credentials.db").exists());
}

#[test]
fn cursors_cover_equal_timestamps_without_duplicates_and_reject_cross_chat_use() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 100).unwrap();
    inbox
        .import_event_page(&json!(
            (0..5)
                .map(|i| event(
                    "123@g.us",
                    "10@s.whatsapp.net",
                    &i.to_string(),
                    1000,
                    json!({"conversation":i.to_string()})
                ))
                .collect::<Vec<_>>()
        ))
        .unwrap();
    let first = inbox.list_messages("123@g.us", None, 2).unwrap();
    let second = inbox
        .list_messages("123@g.us", first.next_cursor.as_deref(), 2)
        .unwrap();
    let third = inbox
        .list_messages("123@g.us", second.next_cursor.as_deref(), 2)
        .unwrap();
    let ids: std::collections::HashSet<_> = first
        .messages
        .iter()
        .chain(&second.messages)
        .chain(&third.messages)
        .map(|m| &m.message_id)
        .collect();
    assert_eq!(ids.len(), 5);
    assert!(third.next_cursor.is_none());
    assert!(
        inbox
            .list_messages("456@g.us", first.next_cursor.as_deref(), 2)
            .is_err()
    );
}

#[test]
fn retention_is_bounded_live_and_search_matches_message_text() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 3).unwrap();
    inbox
        .import_event_page(&json!(
            (0..5)
                .map(|i| event(
                    "123@g.us",
                    "10@s.whatsapp.net",
                    &i.to_string(),
                    i * 1000,
                    json!({"conversation":format!("needle-{i}")})
                ))
                .collect::<Vec<_>>()
        ))
        .unwrap();
    let page = inbox.list_messages("123@g.us", None, 50).unwrap();
    assert_eq!(page.messages.len(), 3);
    assert!(page.coverage.retention_truncated);
    assert_eq!(
        inbox
            .list_chats(Some("needle-4"), None, 50)
            .unwrap()
            .chats
            .len(),
        1
    );
    assert!(
        inbox
            .list_chats(Some("absent"), None, 50)
            .unwrap()
            .chats
            .is_empty()
    );
    inbox.set_limit(1).unwrap();
    assert_eq!(
        inbox
            .list_messages("123@g.us", None, 50)
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn view_once_expired_and_media_secrets_never_enter_archive() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 100).unwrap();
    inbox.import_event_page(&json!([
        event("123@g.us", "10@s.whatsapp.net", "once", 1000, json!({"view_once_message":{"message":{"image_message":{"caption":"private caption","media_key":"secret-key"}}}})),
        event("123@g.us", "10@s.whatsapp.net", "expired", 1000, json!({"ephemeral_message":{"message":{"extended_text_message":{"text":"expired secret","context_info":{"expiration":1}}}}})),
        event("123@g.us", "10@s.whatsapp.net", "photo", 1000, json!({"image_message":{"caption":"shared caption","media_key":"secret-key","url":"private-url"}}))
    ])).unwrap();
    let encoded =
        serde_json::to_string(&inbox.list_messages("123@g.us", None, 50).unwrap()).unwrap();
    assert!(encoded.contains("shared caption"));
    for secret in [
        "private caption",
        "expired secret",
        "secret-key",
        "private-url",
    ] {
        assert!(!encoded.contains(secret), "retained {secret}");
    }
    let bytes = std::fs::read(dir.path().join("inbox.db")).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    for secret in [
        "private caption",
        "expired secret",
        "secret-key",
        "private-url",
    ] {
        assert!(!raw.contains(secret), "persisted {secret}");
    }
}

#[test]
fn edits_and_revocations_survive_old_history_replays_without_touching_other_senders() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 100).unwrap();
    let original = event(
        "123@g.us",
        "10@s.whatsapp.net",
        "original",
        1000,
        json!({"conversation":"old text"}),
    );
    inbox.import_event_page(&json!([
        original.clone(),
        event("123@g.us", "20@s.whatsapp.net", "original", 1000, json!({"conversation":"other sender"})),
        event("123@g.us", "10@s.whatsapp.net", "edit", 2000, json!({"protocol_message":{"type":"MESSAGE_EDIT","key":{"id":"original","participant":"10@s.whatsapp.net"},"edited_message":{"conversation":"updated text"}}})),
        original.clone()
    ])).unwrap();
    let page = inbox.list_messages("123@g.us", None, 50).unwrap();
    assert_eq!(page.messages.len(), 2);
    assert!(page.messages.iter().any(|m| m.text == "updated text"));
    assert!(!page.messages.iter().any(|m| m.text == "old text"));
    inbox.import_event_page(&json!([
        event("123@g.us", "10@s.whatsapp.net", "revoke", 3000, json!({"protocol_message":{"type":"REVOKE","key":{"id":"original","participant":"10@s.whatsapp.net"}}})),
        original
    ])).unwrap();
    let page = inbox.list_messages("123@g.us", None, 50).unwrap();
    assert!(
        page.messages
            .iter()
            .any(|m| m.kind == "deleted" && m.text.is_empty())
    );
    assert!(page.messages.iter().any(|m| m.text == "other sender"));
    assert!(!page.messages.iter().any(|m| m.text == "updated text"));
}

#[test]
fn clearing_account_invalidates_old_captures_and_delete_closes_archive() {
    let dir = TempDir::new().unwrap();
    let inbox = Inbox::open(dir.path(), 100).unwrap();
    inbox
        .import_event_page(&json!([event(
            "123@g.us",
            "10@s.whatsapp.net",
            "a",
            1000,
            json!({"conversation":"account one"})
        )]))
        .unwrap();
    inbox.set_account("first@s.whatsapp.net").unwrap();
    inbox.set_account("second@s.whatsapp.net").unwrap();
    assert!(
        inbox
            .list_messages("123@g.us", None, 50)
            .unwrap()
            .messages
            .is_empty()
    );
    inbox.close().unwrap();
    assert!(inbox.import_event_page(&json!([])).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_archive_is_rejected_and_new_database_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let outside = dir.path().join("outside");
    std::fs::write(&outside, b"untouched").unwrap();
    std::os::unix::fs::symlink(&outside, dir.path().join("inbox.db")).unwrap();
    assert!(Inbox::open(dir.path(), 100).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    std::fs::remove_file(dir.path().join("inbox.db")).unwrap();
    let _inbox = Inbox::open(dir.path(), 100).unwrap();
    assert_eq!(
        std::fs::metadata(dir.path().join("inbox.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
