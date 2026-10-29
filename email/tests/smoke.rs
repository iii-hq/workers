//! Tiny in-process smoke tests. Real IMAP/SMTP integration tests land in PR 1.5
//! with greenmail + mock-smtp containers; these checks just verify config
//! parsing, manifest shape, and the `email::on-new-mail` event shape so
//! refactors don't silently drift.

use email::config::WorkerConfig;
use email::events::new_mail::{NewMailEvent, ON_NEW_MAIL_FN};
use email::manifest::build_manifest;

#[test]
fn manifest_lists_nine_functions_with_private_on_new_mail() {
    let m = build_manifest();
    let fns = m["functions"].as_array().expect("functions array");
    assert_eq!(fns.len(), 9, "expected 9 functions in manifest");
    let ids: Vec<_> = fns.iter().map(|f| f["id"].as_str().unwrap_or("")).collect();
    for expected in [
        "email::send",
        "email::accounts::list",
        "email::list",
        "email::get",
        "email::search",
        "email::flag",
        "email::move",
        "email::attachment::get",
        "email::on-new-mail",
    ] {
        assert!(ids.contains(&expected), "missing {expected}");
    }
    let on_new_mail = fns
        .iter()
        .find(|f| f["id"] == ON_NEW_MAIL_FN)
        .expect("email::on-new-mail entry");
    assert_eq!(on_new_mail["private"], true);
}

#[test]
fn config_defaults_apply() {
    let cfg = WorkerConfig::default();
    assert_eq!(cfg.limits.max_attachment_bytes, 26_214_400);
    assert_eq!(cfg.limits.max_recipients, 100);
    assert_eq!(cfg.limits.send_timeout_ms, 30_000);
    assert_eq!(cfg.limits.imap_connect_timeout_ms, 15_000);
}

#[test]
fn new_mail_event_round_trips_the_wire_shape() {
    let wire = serde_json::json!({
        "account": "support",
        "folder": "INBOX",
        "uid": 12345,
        "message_id": "<abc@mx.example.com>",
        "from": "sender@example.com",
        "subject": "Ticket #42",
        "snippet": "first ~200 chars of body",
        "ts": "2026-05-28T10:14:00+00:00",
    });
    let event: NewMailEvent = serde_json::from_value(wire.clone()).expect("event parses");
    assert_eq!(event.account, "support");
    assert_eq!(event.uid, 12345);
    assert_eq!(serde_json::to_value(&event).unwrap(), wire);
}
