#[tokio::test]
async fn merged_initial_snapshot_persists_final_notification_before_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed(&s);
    s.store()
        .unwrap()
        .change(|d| {
            d.subscribers.insert(
                "s".into(),
                Subscriber {
                    id: "s".into(),
                    function_id: "notify".into(),
                    filter: EventFilter::default(),
                    metadata: None,
                    namespace: Some("target".into()),
                },
            );
            Ok(())
        })
        .unwrap();
    let path = dir.path().join("mock.json");
    std::fs::write(
        path,
        json!({"pr":{"head":{"sha":"current"},"state":"closed","merged":true}}).to_string(),
    )
    .unwrap();
    s.reconcile_watch("w1", "initial").await.unwrap();
    let d = s.store().unwrap().read().unwrap();
    assert_eq!(d.watches["w1"].status, WatchState::Completed);
    assert!(d.repos.contains_key("owner/repo"));
    assert!(d.jobs.values().any(
        |j| matches!(j, Job::Notify { event, .. } if event.final_event && event.kind == "merged")
    ));
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn delete_failure_is_not_reported_as_success() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap();
    s.store()
        .unwrap()
        .change(|d| {
            for w in d.watches.values_mut() {
                w.status = WatchState::Stopped;
            }
            Ok(())
        })
        .unwrap();
    let path = dir.path().join("mock.json");
    let mut d: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    d["fail_delete"] = json!(true);
    std::fs::write(&path, d.to_string()).unwrap();
    assert!(s.cleanup().await.is_err());
    assert_eq!(s.status("w1").unwrap().status, WatchState::CleanupPending);
    assert_eq!(s.status("w1").unwrap().health.cleanup_attempts, 1);
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn wrong_hook_repo_and_oversize_never_enter_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path()).await;
    seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap();
    let raw = br#"{"repository":{"full_name":"foreign/repo"}}"#;
    assert!(matches!(
        s.accept("endpoint", &headers(raw), raw),
        Err(Failure::Signature)
    ));
    let raw = br#"{"repository":{"full_name":"owner/repo"}}"#;
    let mut h = headers(raw);
    h.insert("X-GitHub-Hook-ID".into(), "99".into());
    assert!(matches!(
        s.accept("endpoint", &h, raw),
        Err(Failure::Signature)
    ));
    s.config.max_body_bytes = 1;
    assert!(matches!(
        s.accept("endpoint", &headers(raw), raw),
        Err(Failure::Oversize)
    ));
    assert!(s.store().unwrap().read().unwrap().jobs.is_empty());
    s.iii.shutdown_async().await;
}

use super::*;
use std::collections::{BTreeSet, HashMap};

fn request(id: &str, number: u64) -> WatchRequest {
    serde_json::from_value(json!({"watch_id":id,"repo":"owner/repo","number":number,"expires_at":"2099-01-01T00:00:00Z"})).unwrap()
}
fn watch(id: &str) -> Watch {
    Watch {
        spec: request(id, 1).validate().unwrap(),
        status: WatchState::Active,
        snapshot: Snapshot {
            head_sha: Some("current".into()),
            state: Some("open".into()),
            ..Default::default()
        },
        lease_id: None,
        error: None,
        seen: BTreeSet::new(),
    }
}
fn inbox(event: &str, body: Value) -> Inbox {
    Inbox {
        repo: "owner/repo".into(),
        event: event.into(),
        delivery: "delivery-1".into(),
        body,
    }
}
fn signature(raw: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"test-secret").unwrap();
    mac.update(raw);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}
#[test]
fn hmac_verifies_original_utf8_and_spaces() {
    let raw = "{ \"text\": \"olá 🌍\" }\n".as_bytes();
    assert!(verify_signature("test-secret", raw, &signature(raw)).is_ok());
}
#[test]
fn hmac_rejects_tampered_or_reformatted_json() {
    let raw = b"{ \"key\": 1 }";
    assert!(verify_signature("test-secret", b"{\"key\":1}", &signature(raw)).is_err());
}
#[test]
fn stale_sha_does_not_regress_head_or_ci() {
    let mut w = watch("w");
    let i = inbox(
        "check_run",
        json!({"check_run":{"head_sha":"old","id":7,"status":"completed","conclusion":"success"}}),
    );
    let snapshot = w.snapshot.clone();
    assert!(normalize::normalize(&mut w, &i, snapshot.clone()).is_none());
    assert_eq!(w.snapshot, snapshot);
}
#[test]
fn empty_pr_list_on_fork_correlates_by_sha_without_aggregate_success() {
    let mut w = watch("w");
    let i = inbox(
        "check_run",
        json!({"check_run":{"head_sha":"current","id":7,"status":"completed","conclusion":"success","pull_requests":[]}}),
    );
    assert!(normalize::relevant(&w, &i));
    let snapshot = w.snapshot.clone();
    let event = normalize::normalize(&mut w, &i, snapshot).unwrap();
    assert_eq!(event.snapshot.ci.len(), 1);
    assert_eq!(event.category, Category::Ci);
    assert!(!event.final_event);
}
#[test]
fn issue_comment_ignores_non_pr_issues_and_other_prs() {
    let w = watch("w");
    assert!(!normalize::relevant(
        &w,
        &inbox("issue_comment", json!({"issue":{"number":1}}))
    ));
    assert!(!normalize::relevant(
        &w,
        &inbox(
            "issue_comment",
            json!({"issue":{"number":2,"pull_request":{} }})
        )
    ));
    assert!(normalize::relevant(
        &w,
        &inbox(
            "issue_comment",
            json!({"issue":{"number":1,"pull_request":{} }})
        )
    ));
}
#[test]
fn closed_without_merge_keeps_merged_watch_alive() {
    let mut w = watch("w");
    let i = inbox("pull_request", json!({"action":"closed","number":1}));
    let mut snapshot = w.snapshot.clone();
    snapshot.state = Some("closed".into());
    let event = normalize::normalize(&mut w, &i, snapshot).unwrap();
    assert!(!event.final_event);
    assert!(w.live());
}
#[test]
fn closed_mode_and_initial_merge_resolve() {
    let mut closed = watch("closed");
    closed.spec.stop_on = StopOn::Closed;
    closed.snapshot.state = Some("closed".into());
    assert!(normalize::finish_if_needed(&mut closed));
    let mut merged = watch("merged");
    merged.snapshot.merged = true;
    assert!(normalize::finish_if_needed(&mut merged));
    assert_eq!(merged.status, WatchState::Completed);
}
#[test]
fn duplicate_entity_delivery_has_no_second_effect() {
    let mut w = watch("w");
    let i = inbox(
        "issue_comment",
        json!({"action":"created","issue":{"number":1,"pull_request":{}},"comment":{"id":8,"body":"hi"}}),
    );
    let snapshot = w.snapshot.clone();
    assert!(normalize::normalize(&mut w, &i, snapshot.clone()).is_some());
    assert!(normalize::normalize(&mut w, &i, snapshot).is_none());
}
#[test]
fn filter_and_subscriber_metadata_namespace_survive_outbox() {
    let mut d = Data::default();
    let sub = Subscriber {
        id: "s".into(),
        function_id: "notify".into(),
        filter: EventFilter {
            watch_id: Some("w".into()),
            categories: Some(BTreeSet::from([Category::Pr])),
            ..Default::default()
        },
        metadata: Some(json!({"tenant":3})),
        namespace: Some("another".into()),
    };
    d.subscribers.insert("s".into(), sub);
    let event = normalize::make_event(&watch("w"), Category::Pr, "merged", "pr", 0, true);
    normalize::persist_event(&mut d, event, "source");
    let Job::Notify { target, .. } = d.jobs.values().next().unwrap() else {
        panic!("notification required")
    };
    assert_eq!(target.metadata, Some(json!({"tenant":3})));
    assert_eq!(target.namespace.as_deref(), Some("another"));
}
#[test]
fn storage_reopens_inbox_and_rejects_second_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let s = Store::open(&path).unwrap();
    s.change(|d| {
        d.jobs.insert(
            "inbox:1".into(),
            Job::Inbox(inbox("pull_request", json!({}))),
        );
        Ok(())
    })
    .unwrap();
    assert!(Store::open(&path).is_err());
    drop(s);
    assert!(Store::open(&path)
        .unwrap()
        .read()
        .unwrap()
        .jobs
        .contains_key("inbox:1"));
}
#[test]
fn failed_transaction_does_not_commit_dedupe_without_job() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("store.sqlite3")).unwrap();
    let result: Result<()> = s.change(|d| {
        d.deliveries.insert("42:delivery".into());
        Err(Failure::Capacity)
    });
    assert!(result.is_err());
    assert!(s.read().unwrap().deliveries.is_empty());
}
#[test]
fn watch_selector_is_canonical_and_strict() {
    let a = request("same", 1).validate().unwrap();
    let b: WatchRequest = serde_json::from_value(json!({"watch_id":"same","pr_url":"https://github.com/OWNER/REPO/pull/1","expires_at":"2099-01-01T00:00:00Z"})).unwrap();
    assert_eq!(a, b.validate().unwrap());
    let mut invalid = request("same", 1);
    invalid.pr_url = Some("https://github.com/owner/repo/pull/1".into());
    assert!(invalid.validate().is_err());
}

pub(super) async fn service(dir: &std::path::Path) -> Service {
    let path = dir.join("mock.json");
    std::fs::write(
        &path,
        json!({"pr":{"head":{"sha":"current"},"state":"open","merged":false}}).to_string(),
    )
    .unwrap();
    // Isolate the executable and its mode from concurrent fixture edits.
    let executable = dir.join("mock-gh.py");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock-gh.py"),
        &executable,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let cfg = crate::config::Config {
        gh_executable: executable.display().to_string(),
        token: path.display().to_string(),
        ..Default::default()
    };
    Service {
        iii: Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        )),
        cell: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
        config: WebhookConfig {
            enabled: true,
            ..Default::default()
        },
        engine_url: "ws://127.0.0.1:1".into(),
        store: Some(Store::open(&dir.join("store.sqlite3")).unwrap()),
        operations: Mutex::new(()),
        bus: None,
    }
}
pub(super) fn seed(s: &Service) {
    s.store()
        .unwrap()
        .change(|d| {
            d.watches.insert("w1".into(), watch("w1"));
            let mut second = watch("w2");
            second.spec.number = 2;
            d.watches.insert("w2".into(), second);
            d.repos.insert(
                "owner/repo".into(),
                RepoHook {
                    endpoint_id: "endpoint".into(),
                    secret: "test-secret".into(),
                    hook_id: None,
                    url: None,
                    pending_url: None,
                    generation: None,
                    create_started: false,
                    cleanup_attempts: 0,
                    error: None,
                },
            );
            Ok(())
        })
        .unwrap();
}
fn headers(raw: &[u8]) -> HashMap<String, String> {
    HashMap::from([
        ("X-Hub-Signature-256".into(), signature(raw)),
        ("X-GitHub-Hook-ID".into(), "42".into()),
        ("X-GitHub-Delivery".into(), "delivery-1".into()),
        ("X-GitHub-Event".into(), "pull_request".into()),
    ])
}
#[tokio::test]
async fn two_prs_share_hook_and_rotation_preserves_secret() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed(&s);
    assert!(s
        .ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap());
    assert!(!s
        .ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap());
    assert!(s
        .ensure_hook("owner/repo", "https://two.example", "2")
        .await
        .unwrap());
    let data: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("mock.json")).unwrap()).unwrap();
    let calls = data["calls"].as_array().unwrap();
    assert_eq!(calls.iter().filter(|c| c["method"] == "POST").count(), 1);
    assert_eq!(data["hook"]["config"]["secret"], "test-secret");
    s.iii.shutdown_async().await;
}
#[tokio::test]
async fn signed_inbox_duplicate_and_queue_failure_retain_work() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap();
    let raw = br#"{ "repository":{"full_name":"owner/repo"}, "number":1 }"#;
    assert!(s.accept("endpoint", &headers(raw), raw).unwrap());
    assert!(!s.accept("endpoint", &headers(raw), raw).unwrap());
    assert!(s.publish_pending().await.is_err());
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.jobs.len(), 1);
    assert!(data.last_error.is_some());
    s.iii.shutdown_async().await;
}
#[tokio::test]
async fn cleanup_failure_remains_pending_and_does_not_delete_foreign_hook() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap();
    s.store()
        .unwrap()
        .change(|d| {
            for w in d.watches.values_mut() {
                w.status = WatchState::Stopped;
            }
            Ok(())
        })
        .unwrap();
    let path = dir.path().join("mock.json");
    let mut data: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    data["hook"]["config"]["url"] = json!("https://foreign.example");
    std::fs::write(path, data.to_string()).unwrap();
    assert!(matches!(s.cleanup().await, Err(Failure::Ownership)));
    assert_eq!(s.status("w1").unwrap().status, WatchState::CleanupPending);
    assert!(s
        .store()
        .unwrap()
        .read()
        .unwrap()
        .repos
        .contains_key("owner/repo"));
    s.iii.shutdown_async().await;
}
#[tokio::test]
async fn snapshot_race_buffers_delivery_until_authoritative_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "1")
        .await
        .unwrap();
    let raw = br#"{"repository":{"full_name":"owner/repo"},"number":1,"action":"synchronize","pull_request":{"head":{"sha":"old"}}}"#;
    s.accept("endpoint", &headers(raw), raw).unwrap();
    s.process("inbox:42:delivery-1").await.unwrap();
    assert_eq!(
        s.status("w1").unwrap().snapshot.head_sha.as_deref(),
        Some("current")
    );
    assert!(s.store().unwrap().read().unwrap().jobs.is_empty());
    s.iii.shutdown_async().await;
}
#[tokio::test]
async fn expiry_is_persisted_without_pr_polling() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    s.store()
        .unwrap()
        .change(|d| {
            let mut w = watch("expired");
            w.spec.expires_at = Utc::now() - chrono::Duration::seconds(1);
            d.watches.insert("expired".into(), w);
            Ok(())
        })
        .unwrap();
    s.maintain().await.unwrap();
    assert_eq!(s.status("expired").unwrap().status, WatchState::Expired);
    let data: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("mock.json")).unwrap()).unwrap();
    assert!(data.get("calls").is_none());
    s.iii.shutdown_async().await;
}
