//! Tiny in-process smoke tests. Real IMAP/SMTP integration tests land in PR 1.5
//! with greenmail + mock-smtp containers; these checks just verify config
//! parsing, manifest shape, and trigger-registry semantics so refactors don't
//! silently drift.

use email::config::WorkerConfig;
use email::manifest::build_manifest;
use email::triggers::registry::TriggerRegistry;

#[test]
fn manifest_lists_eight_functions_and_new_mail_trigger() {
    let m = build_manifest();
    let fns = m["functions"].as_array().expect("functions array");
    assert_eq!(fns.len(), 8, "expected 8 functions in manifest");
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
    ] {
        assert!(ids.contains(&expected), "missing {expected}");
    }
    let triggers = m["trigger_types"].as_array().expect("trigger_types array");
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0]["name"], "email::new-mail");
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
fn registry_fan_out_keying() {
    let reg = TriggerRegistry::new();
    reg.register(
        "support".into(),
        "INBOX".into(),
        "trig-1".into(),
        "h::on_mail".into(),
        30_000,
    );
    reg.register(
        "support".into(),
        "INBOX".into(),
        "trig-2".into(),
        "audit::log".into(),
        5_000,
    );
    reg.register(
        "billing".into(),
        "INBOX".into(),
        "trig-3".into(),
        "billing::ingest".into(),
        30_000,
    );

    let subs = reg.subscribers_for("support", "INBOX");
    assert_eq!(subs.len(), 2);
    let fns: Vec<_> = subs.iter().map(|s| s.function_id.as_str()).collect();
    assert!(fns.contains(&"h::on_mail"));
    assert!(fns.contains(&"audit::log"));

    reg.unregister("trig-1");
    let subs = reg.subscribers_for("support", "INBOX");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].function_id, "audit::log");

    let billing = reg.subscribers_for("billing", "INBOX");
    assert_eq!(billing.len(), 1);
}
