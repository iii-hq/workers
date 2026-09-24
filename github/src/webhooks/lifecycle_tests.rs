#[path = "lifecycle_review_tests.rs"]
mod review;

use super::*;
use std::{collections::VecDeque, sync::Mutex as StdMutex};

#[derive(Clone, Debug)]
struct Call {
    function: String,
    payload: Value,
    namespace: Option<String>,
    metadata: Option<Value>,
}
#[derive(Default)]
pub(super) struct MockBus {
    calls: StdMutex<Vec<Call>>,
    replies: StdMutex<VecDeque<(&'static str, Result<Value>)>>,
}
impl MockBus {
    fn reply(&self, function: &'static str, reply: Result<Value>) {
        self.replies.lock().unwrap().push_back((function, reply));
    }
    pub(super) fn invoke(
        &self,
        function: &str,
        payload: Value,
        namespace: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<Value> {
        self.calls.lock().unwrap().push(Call {
            function: function.into(),
            payload,
            namespace: namespace.map(str::to_owned),
            metadata,
        });
        let (expected, reply) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected bus invocation");
        assert_eq!(function, expected);
        reply
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}
fn offline() -> Failure {
    Failure::Invalid("mock dependency unavailable".into())
}
fn tunnel(status: &str, url: Option<&str>, generation: &str) -> TunnelSnapshot {
    TunnelSnapshot {
        tunnel_id: "webhooks".into(),
        status: status.into(),
        public_url: url.map(str::to_owned),
        generation: generation.into(),
        error: None,
    }
}
async fn service(dir: &Path) -> Service {
    let mut s = tests::service(dir).await;
    s.bus = Some(MockBus::default());
    s
}
fn subscriber(s: &Service, namespace: Option<&str>) {
    s.store()
        .unwrap()
        .change(|d| {
            d.subscribers.insert(
                "s".into(),
                Subscriber {
                    id: "s".into(),
                    function_id: "notify".into(),
                    filter: EventFilter::default(),
                    metadata: Some(json!({"tenant":7})),
                    namespace: namespace.map(str::to_owned),
                },
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn max_length_consumer_id_is_stable_bounded_and_unambiguous() {
    let installation = "a".repeat(64);
    let watch_id = "b".repeat(128);
    let id = lifecycle::consumer_id(&installation, &watch_id);
    assert_eq!(id.len(), 71);
    assert_eq!(id, lifecycle::consumer_id(&installation, &watch_id));
    assert_ne!(id, lifecycle::consumer_id(&"c".repeat(64), &watch_id));
    assert_ne!(
        lifecycle::consumer_id("ab", "c"),
        lifecycle::consumer_id("a", "bc")
    );
}

#[tokio::test]
async fn expiry_over_thirty_days_is_rejected_before_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    let request = serde_json::from_value(json!({
        "watch_id":"new", "repo":"owner/repo", "number":1,
        "expires_at": Utc::now() + chrono::Duration::days(31),
    }))
    .unwrap();
    let error = s.watch(request).await.unwrap_err();
    assert!(error.to_string().contains("30 days"));
    let data = s.store().unwrap().read().unwrap();
    assert!(data.watches.is_empty() && data.repos.is_empty());
    assert!(s.bus.as_ref().unwrap().calls().is_empty());
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn tunnel_callback_is_durable_without_waiting_for_operations_or_network() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    let guard = s.operations.lock().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        s.tunnel_changed(tunnel("ready", Some("https://old.example"), "old")),
    )
    .await
    .unwrap()
    .unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.jobs.len(), 1);
    assert!(s.bus.as_ref().unwrap().calls().is_empty());
    drop(guard);
    s.iii.shutdown_async().await;
    drop(s);
    // Read committed bytes without reclaiming the installation lock: parallel
    // subprocess tests can transiently inherit its flock between fork and exec.
    let persisted = Store::inspect(&dir.path().join("store.sqlite3")).unwrap();
    assert_eq!(persisted.jobs.len(), 1);
}

#[tokio::test]
async fn delayed_ready_and_failure_never_regress_current_hook_url_or_generation() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    // Deliberately lexically smaller current generation: UUIDs are not ordered.
    s.ensure_hook("owner/repo", "https://current.example", "000-current")
        .await
        .unwrap();
    let current = tunnel("ready", Some("https://current.example"), "000-current");
    for stale in [
        tunnel("ready", Some("https://old.example"), "fff-old"),
        tunnel("failed", None, "fff-old"),
    ] {
        s.bus.as_ref().unwrap().reply(
            "quick-tunnel::status",
            Ok(serde_json::to_value(&current).unwrap()),
        );
        s.process_tunnel(&stale).await.unwrap();
    }
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(
        data.repos["owner/repo"].url.as_deref(),
        Some("https://current.example/webhooks/github/endpoint")
    );
    assert_eq!(
        data.repos["owner/repo"].generation.as_deref(),
        Some("000-current")
    );
    assert_eq!(data.tunnel_status, "ready");
    assert_eq!(s.bus.as_ref().unwrap().calls().len(), 2);
    let gh: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("mock.json")).unwrap()).unwrap();
    assert_eq!(
        gh["calls"].as_array().unwrap().len(),
        1,
        "no stale PATCH or PR GET"
    );
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn queued_tunnel_job_uses_authoritative_status_and_acknowledges_only_success() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    s.tunnel_changed(tunnel("ready", Some("https://old.example"), "old"))
        .await
        .unwrap();
    let id = s
        .store()
        .unwrap()
        .read()
        .unwrap()
        .jobs
        .keys()
        .next()
        .unwrap()
        .clone();
    s.bus
        .as_ref()
        .unwrap()
        .reply("quick-tunnel::status", Err(offline()));
    assert!(s.process(&id).await.is_err());
    assert!(s.store().unwrap().read().unwrap().jobs.contains_key(&id));
    s.bus.as_ref().unwrap().reply(
        "quick-tunnel::status",
        Ok(serde_json::to_value(tunnel("reconnecting", None, "current")).unwrap()),
    );
    s.process(&id).await.unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.tunnel_status, "reconnecting");
    assert!(data.jobs.is_empty());
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn notification_none_namespace_routes_default_and_preserves_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.iii.set_namespace("provider");
    subscriber(&s, None);
    s.store()
        .unwrap()
        .change(|d| {
            let event =
                normalize::make_event(&d.watches["w1"], Category::Pr, "snapshot", "pr", 0, false);
            normalize::persist_event(d, event, "test");
            Ok(())
        })
        .unwrap();
    let id = s
        .store()
        .unwrap()
        .read()
        .unwrap()
        .jobs
        .keys()
        .next()
        .unwrap()
        .clone();
    s.bus.as_ref().unwrap().reply("notify", Ok(Value::Null));
    s.process(&id).await.unwrap();
    let calls = s.bus.as_ref().unwrap().calls();
    assert_eq!(calls[0].namespace.as_deref(), Some("default"));
    assert_eq!(calls[0].metadata, Some(json!({"tenant":7})));
    assert_eq!(calls[0].payload["watch_id"], "w1");
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn merged_fallback_publishes_and_cleans_up_even_without_queue() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    subscriber(&s, None);
    std::fs::write(
        dir.path().join("mock.json"),
        json!({"pr":{"head":{"sha":"current"},"state":"closed","merged":true}}).to_string(),
    )
    .unwrap();
    let bus = s.bus.as_ref().unwrap();
    bus.reply("quick-tunnel::acquire", Err(offline()));
    bus.reply("iii::durable::publish", Err(offline()));
    let request = serde_json::from_value(json!({"watch_id":"merged","repo":"owner/repo","number":1,"expires_at":Utc::now()+chrono::Duration::days(1)})).unwrap();
    assert!(s.watch(request).await.is_err());
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.watches["merged"].status, WatchState::Completed);
    assert!(data.watches["merged"].error.is_some());
    assert!(
        data.repos.is_empty(),
        "cleanup must run despite queue failure"
    );
    assert!(data.jobs.values().any(|job| matches!(job, Job::Notify { event, .. } if event.final_event && event.kind == "merged")));
    assert_eq!(
        bus.calls()
            .iter()
            .filter(|c| c.function == "iii::durable::publish")
            .count(),
        1
    );
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn starting_snapshot_stays_preparing_and_failed_hook_never_looks_ready() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.apply_tunnel(&tunnel("starting", None, "one"))
        .await
        .unwrap();
    s.reconcile_watch("w1", "initial").await.unwrap();
    assert_eq!(s.status("w1").unwrap().status, WatchState::Preparing);
    s.ensure_hook("owner/repo", "https://current.example", "one")
        .await
        .unwrap();
    s.store()
        .unwrap()
        .change(|d| {
            d.tunnel_status = "ready".into();
            Ok(())
        })
        .unwrap();
    s.reconcile_watch("w1", "hook-ready").await.unwrap();
    assert!(s.status("w1").unwrap().health.hook_ready);
    s.store()
        .unwrap()
        .change(|d| {
            d.repos.get_mut("owner/repo").unwrap().error = Some("patch failed".into());
            Ok(())
        })
        .unwrap();
    assert!(!s.status("w1").unwrap().health.hook_ready);
    s.apply_tunnel(&tunnel("failed", None, "two"))
        .await
        .unwrap();
    assert_eq!(s.status("w1").unwrap().status, WatchState::Preparing);
    assert_eq!(s.status("w1").unwrap().health.tunnel_status, "failed");
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn cleanup_failure_does_not_block_other_repositories_or_releases() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.store()
        .unwrap()
        .change(|d| {
            let mut good = d.repos["owner/repo"].clone();
            good.endpoint_id = "other".into();
            d.repos.insert("z/good".into(), good);
            d.repos.get_mut("owner/repo").unwrap().create_started = true;
            d.watches.get_mut("w1").unwrap().status = WatchState::Stopped;
            let w = d.watches.get_mut("w2").unwrap();
            w.status = WatchState::Stopped;
            w.spec.repo = "z/good".into();
            w.lease_id = Some("good-lease".into());
            Ok(())
        })
        .unwrap();
    s.bus
        .as_ref()
        .unwrap()
        .reply("quick-tunnel::release", Ok(Value::Null));
    assert!(matches!(s.cleanup().await, Err(Failure::AmbiguousHook)));
    let data = s.store().unwrap().read().unwrap();
    assert!(data.repos.contains_key("owner/repo"));
    assert!(!data.repos.contains_key("z/good"));
    assert!(data.watches["w2"].lease_id.is_none());
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn release_failure_does_not_block_next_watch_and_closed_retry_restores_completed() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.store()
        .unwrap()
        .change(|d| {
            d.repos.clear();
            for (id, w) in &mut d.watches {
                w.spec.stop_on = StopOn::Closed;
                w.snapshot.state = Some("closed".into());
                w.status = WatchState::Completed;
                w.lease_id = Some(id.clone());
            }
            Ok(())
        })
        .unwrap();
    let bus = s.bus.as_ref().unwrap();
    bus.reply("quick-tunnel::release", Err(offline()));
    bus.reply("quick-tunnel::release", Ok(Value::Null));
    assert!(s.cleanup().await.is_err());
    assert_eq!(s.status("w1").unwrap().status, WatchState::CleanupPending);
    assert!(s.store().unwrap().read().unwrap().watches["w2"]
        .lease_id
        .is_none());
    bus.reply("quick-tunnel::release", Ok(Value::Null));
    s.cleanup().await.unwrap();
    assert_eq!(s.status("w1").unwrap().status, WatchState::Completed);
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn recover_waits_for_lock_before_reset_and_does_not_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    s.store()
        .unwrap()
        .change(|d| {
            d.publications.insert("pending".into(), (0, 5));
            Ok(())
        })
        .unwrap();
    let guard = s.operations.lock().await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), s.recover(None))
            .await
            .is_err()
    );
    assert!(s
        .store()
        .unwrap()
        .read()
        .unwrap()
        .publications
        .contains_key("pending"));
    drop(guard);
    tokio::time::timeout(std::time::Duration::from_secs(1), s.recover(None))
        .await
        .unwrap()
        .unwrap();
    assert!(s.store().unwrap().read().unwrap().publications.is_empty());
    s.iii.shutdown_async().await;
}

#[test]
fn defaults_are_valid_but_zero_capacity_is_rejected() {
    let mut config = WebhookConfig::default();
    assert!(wiring::validate_config(&config).is_ok());
    config.max_pending = 0;
    assert!(wiring::validate_config(&config).is_err());
}

async fn set_grace(s: &Service, minutes: u32) {
    let mut config = (**s.cell.read().await).clone();
    config.webhooks.orphan_grace_minutes = minutes;
    *s.cell.write().await = std::sync::Arc::new(config);
}
fn listen(d: &mut Data, id: &str, watch_id: &str) {
    d.subscribers.insert(
        id.into(),
        Subscriber {
            id: id.into(),
            function_id: "notify".into(),
            filter: EventFilter {
                watch_id: Some(watch_id.into()),
                ..Default::default()
            },
            metadata: None,
            namespace: None,
        },
    );
}

#[tokio::test]
async fn zero_grace_stops_watch_releases_lease_and_deletes_hook_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    set_grace(&s, 0).await;
    tests::seed(&s);
    s.store()
        .unwrap()
        .change(|d| {
            let w1 = d.watches.get_mut("w1").unwrap();
            w1.status = WatchState::Active;
            w1.lease_id = Some("lease-w1".into());
            let w2 = d.watches.get_mut("w2").unwrap();
            w2.status = WatchState::Stopped;
            w2.lease_id = None;
            listen(d, "chat", "w1");
            Ok(())
        })
        .unwrap();
    s.bus
        .as_ref()
        .unwrap()
        .reply("quick-tunnel::release", Ok(Value::Null));
    let stopped = s.drop_subscribers(&["chat".into()]).await.unwrap();
    assert_eq!(stopped, ["w1"]);
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.watches["w1"].status, WatchState::Stopped);
    assert!(data.watches["w1"].lease_id.is_none(), "lease released");
    assert!(data.subscribers.is_empty());
    assert!(data.repos.is_empty(), "no live watch left: hook deleted");
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn a_remaining_listener_keeps_the_watch_lease_and_hook() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.store()
        .unwrap()
        .change(|d| {
            let w1 = d.watches.get_mut("w1").unwrap();
            w1.status = WatchState::Active;
            w1.lease_id = Some("lease-w1".into());
            listen(d, "chat", "w1");
            listen(d, "other-chat", "w1");
            Ok(())
        })
        .unwrap();
    // No bus reply queued: any release or hook call would fail the test.
    assert!(s
        .drop_subscribers(&["chat".into()])
        .await
        .unwrap()
        .is_empty());
    let data = s.store().unwrap().read().unwrap();
    assert!(data.watches["w1"].live());
    assert_eq!(data.watches["w1"].lease_id.as_deref(), Some("lease-w1"));
    assert!(data.repos.contains_key("owner/repo"));
    assert!(data.subscribers.contains_key("other-chat"));
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn watch_expiry_follows_configured_max_watch_days() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    let mut config = (**s.cell.read().await).clone();
    config.webhooks.max_watch_days = 2;
    *s.cell.write().await = std::sync::Arc::new(config);
    let request = serde_json::from_value(json!({"watch_id":"long","repo":"owner/repo","number":1,"expires_at":Utc::now()+chrono::Duration::days(3)})).unwrap();
    let error = s.watch(request).await.unwrap_err().to_string();
    assert!(error.contains("at most 2 days"), "{error}");
    assert!(error.contains("quick-tunnel"));
    assert!(!crate::webhooks::valid_watch_days(0));
    assert!(!crate::webhooks::valid_watch_days(31));
    assert!(crate::webhooks::valid_watch_days(30));
    s.iii.shutdown_async().await;
}

fn seed_one_listener(s: &Service) {
    tests::seed(s);
    s.store()
        .unwrap()
        .change(|d| {
            let w1 = d.watches.get_mut("w1").unwrap();
            w1.status = WatchState::Active;
            w1.lease_id = Some("lease-w1".into());
            let w2 = d.watches.get_mut("w2").unwrap();
            w2.status = WatchState::Stopped;
            w2.lease_id = None;
            listen(d, "wake", "w1");
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn one_shot_wake_leaving_only_marks_and_a_rearm_keeps_the_watch() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed_one_listener(&s);
    // Default grace: the fired one-shot wake leaves, nothing is released.
    assert_eq!(s.drop_subscribers(&["wake".into()]).await.unwrap(), ["w1"]);
    let data = s.store().unwrap().read().unwrap();
    assert!(data.watches["w1"].live());
    assert!(data.watches["w1"].orphaned_at.is_some());
    assert_eq!(data.watches["w1"].lease_id.as_deref(), Some("lease-w1"));
    // The agent re-arms at the end of its turn: maintenance clears the mark.
    s.store()
        .unwrap()
        .change(|d| {
            listen(d, "wake-2", "w1");
            Ok(())
        })
        .unwrap();
    s.maintain().await.unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert!(data.watches["w1"].live());
    assert!(data.watches["w1"].orphaned_at.is_none());
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn watch_without_listeners_past_grace_is_stopped_and_released() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    seed_one_listener(&s);
    s.drop_subscribers(&["wake".into()]).await.unwrap();
    s.store()
        .unwrap()
        .change(|d| {
            d.watches.get_mut("w1").unwrap().orphaned_at =
                Some(Utc::now() - chrono::Duration::minutes(61));
            Ok(())
        })
        .unwrap();
    s.bus
        .as_ref()
        .unwrap()
        .reply("quick-tunnel::release", Ok(Value::Null));
    s.maintain().await.unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.watches["w1"].status, WatchState::Stopped);
    assert!(data.watches["w1"].lease_id.is_none());
    assert!(data.repos.is_empty(), "last live watch gone: hook deleted");
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn rewatching_the_same_spec_resumes_a_stopped_watch() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    let expires_at = Utc::now() + chrono::Duration::days(1);
    let request = || {
        serde_json::from_value::<WatchRequest>(
            json!({"watch_id":"resume","repo":"owner/repo","number":1,"expires_at":expires_at}),
        )
        .unwrap()
    };
    let bus = s.bus.as_ref().unwrap();
    bus.reply("quick-tunnel::acquire", Err(offline()));
    let _ = s.watch(request()).await;
    s.store()
        .unwrap()
        .change(|d| {
            d.watches.get_mut("resume").unwrap().status = WatchState::Stopped;
            d.repos.remove("owner/repo");
            Ok(())
        })
        .unwrap();
    bus.reply("quick-tunnel::acquire", Err(offline()));
    let _ = s.watch(request()).await;
    let data = s.store().unwrap().read().unwrap();
    assert!(data.watches["resume"].live(), "stopped watch resumed");
    assert!(
        data.repos.contains_key("owner/repo"),
        "repository hook re-planned"
    );
    s.iii.shutdown_async().await;
}
