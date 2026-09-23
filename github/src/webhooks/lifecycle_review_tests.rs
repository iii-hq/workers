//! Regression coverage for PR #1208 lifecycle review; all state is temporary.
use super::*;

fn gh_state(dir: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(dir.join("mock.json")).unwrap()).unwrap()
}
fn configure_gh(dir: &Path, fields: Value) {
    let mut state = gh_state(dir);
    state
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    std::fs::write(dir.join("mock.json"), state.to_string()).unwrap();
}
fn gh_calls(dir: &Path, method: &str) -> Vec<Value> {
    gh_state(dir)["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["method"] == method)
        .cloned()
        .collect()
}
fn stop_watches(s: &Service) {
    s.store()
        .unwrap()
        .change(|d| {
            for w in d.watches.values_mut() {
                w.status = WatchState::Stopped;
            }
            Ok(())
        })
        .unwrap();
}
fn crash_intent(s: &Service) {
    s.store()
        .unwrap()
        .change(|d| {
            let hook = d.repos.get_mut("owner/repo").unwrap();
            hook.create_started = true;
            hook.url = Some("https://old.example/webhooks/github/endpoint".into());
            Ok(())
        })
        .unwrap();
}

/// Reclaim the temporary installation after forked test children have exec'd.
async fn reopen_store(s: &mut Service, dir: &Path) {
    s.store = None;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match Store::open(&dir.join("store.sqlite3")) {
            Ok(store) => {
                s.store = Some(store);
                return;
            }
            // Parallel gh tests can inherit flock until exec closes their copy.
            // Retry only that transient ownership error; never ignore corruption.
            Err(Failure::Invalid(message))
                if message == "another process owns this webhook installation"
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => panic!("failed to reopen temporary webhook store: {error}"),
        }
    }
}

/// Model a rejected PATCH or an applied PATCH whose response was lost.
async fn interrupted_rotation(s: &Service, dir: &Path, applied: bool) {
    s.ensure_hook("owner/repo", "https://one.example", "one")
        .await
        .unwrap();
    s.store()
        .unwrap()
        .change(|d| {
            d.tunnel_status = "ready".into();
            for watch in d.watches.values_mut() {
                watch.status = WatchState::Active;
                watch.lease_id = Some(format!("lease-{}", watch.spec.watch_id));
            }
            Ok(())
        })
        .unwrap();
    configure_gh(dir, json!({"fail_patch":true,"patch_applied":applied}));
    assert!(s
        .ensure_hook("owner/repo", "https://two.example", "two")
        .await
        .is_err());
    let data = s.store().unwrap().read().unwrap();
    let hook = &data.repos["owner/repo"];
    assert_eq!(
        hook.url.as_deref(),
        Some("https://one.example/webhooks/github/endpoint")
    );
    assert_eq!(
        hook.pending_url.as_deref(),
        Some("https://two.example/webhooks/github/endpoint")
    );
    assert!(hook.generation.is_none());
    assert!(hook.error.is_some());
    assert!(!s.status("w1").unwrap().health.hook_ready);
}

#[tokio::test]
async fn lost_patch_response_recovers_after_restart_and_another_url_change() {
    for applied in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = service(dir.path()).await;
        tests::seed(&s);
        interrupted_rotation(&s, dir.path(), applied).await;
        reopen_store(&mut s, dir.path()).await;
        configure_gh(dir.path(), json!({"fail_patch":false}));
        s.apply_tunnel(&tunnel("ready", Some("https://three.example"), "three"))
            .await
            .unwrap();
        let data = s.store().unwrap().read().unwrap();
        let hook = &data.repos["owner/repo"];
        assert_eq!(hook.hook_id, Some(42));
        assert_eq!(
            hook.url.as_deref(),
            Some("https://three.example/webhooks/github/endpoint")
        );
        assert_eq!(hook.generation.as_deref(), Some("three"));
        assert!(hook.pending_url.is_none());
        assert!(hook.error.is_none());
        for id in ["w1", "w2"] {
            let status = s.status(id).unwrap();
            assert_eq!(status.status, WatchState::Active);
            assert!(status.health.hook_ready);
        }
        assert_eq!(gh_calls(dir.path(), "POST").len(), 1);
        assert_eq!(gh_calls(dir.path(), "PATCH").len(), 2);
        assert_eq!(
            gh_state(dir.path())["hook"]["config"]["secret"],
            "test-secret"
        );
        s.iii.shutdown_async().await;
    }
}

#[tokio::test]
async fn lost_patch_response_allows_cleanup_after_restart_without_releasing_foreign_hooks() {
    for applied in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = service(dir.path()).await;
        tests::seed(&s);
        interrupted_rotation(&s, dir.path(), applied).await;
        reopen_store(&mut s, dir.path()).await;
        stop_watches(&s);
        let bus = s.bus.as_ref().unwrap();
        for _ in 0..2 {
            bus.reply("quick-tunnel::release", Ok(Value::Null));
        }
        s.cleanup().await.unwrap();
        let data = s.store().unwrap().read().unwrap();
        assert!(data.repos.is_empty());
        assert!(data.watches.values().all(|w| w.lease_id.is_none()));
        assert_eq!(gh_calls(dir.path(), "DELETE").len(), 1);
        assert_eq!(gh_calls(dir.path(), "PATCH").len(), 1);
        assert_eq!(bus.calls()[0].payload["lease_id"], "lease-w1");
        assert_eq!(bus.calls()[1].payload["lease_id"], "lease-w2");
        s.iii.shutdown_async().await;
    }
}

#[tokio::test]
async fn pending_patch_does_not_authorize_unknown_urls_ids_or_failed_reads() {
    for failure in ["url", "id", "read"] {
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path()).await;
        tests::seed(&s);
        interrupted_rotation(&s, dir.path(), true).await;
        let mut state = gh_state(dir.path());
        match failure {
            "url" => {
                state["hook"]["config"]["url"] =
                    json!("https://foreign.example/webhooks/github/endpoint")
            }
            "id" => state["hook"]["id"] = json!(99),
            _ => state["fail_get_hook"] = json!(true),
        }
        std::fs::write(dir.path().join("mock.json"), state.to_string()).unwrap();
        assert!(s
            .ensure_hook("owner/repo", "https://three.example", "three")
            .await
            .is_err());
        stop_watches(&s);
        assert!(s.cleanup().await.is_err());
        let data = s.store().unwrap().read().unwrap();
        assert_eq!(
            data.repos["owner/repo"].pending_url.as_deref(),
            Some("https://two.example/webhooks/github/endpoint")
        );
        assert!(data.watches.values().all(|w| w.lease_id.is_some()));
        assert!(gh_calls(dir.path(), "DELETE").is_empty());
        assert_eq!(gh_calls(dir.path(), "PATCH").len(), 1);
        assert!(s.bus.as_ref().unwrap().calls().is_empty());
        s.iii.shutdown_async().await;
    }
}

#[test]
fn legacy_hook_without_update_intent_remains_readable() {
    let hook: RepoHook = serde_json::from_value(json!({
        "endpoint_id":"endpoint", "secret":"test-secret", "hook_id":42,
        "url":"https://one.example/webhooks/github/endpoint", "generation":"one",
        "create_started":false, "cleanup_attempts":0, "error":null
    }))
    .unwrap();
    assert!(hook.pending_url.is_none());
    assert!(serde_json::to_value(hook)
        .unwrap()
        .get("pending_url")
        .is_none());
}

#[tokio::test]
async fn rejected_post_without_hook_clears_intent_and_releases_lease() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    configure_gh(dir.path(), json!({"fail_create":true}));
    assert!(s
        .ensure_hook("owner/repo", "https://one.example", "one")
        .await
        .is_err());
    let data = s.store().unwrap().read().unwrap();
    assert!(!data.repos["owner/repo"].create_started);
    assert!(data.repos["owner/repo"].hook_id.is_none());
    assert_eq!(
        gh_calls(dir.path(), "GET")[0]["endpoint"],
        "repos/owner/repo/hooks?per_page=100"
    );
    s.store()
        .unwrap()
        .change(|d| {
            d.watches.get_mut("w1").unwrap().lease_id = Some("old-lease".into());
            Ok(())
        })
        .unwrap();
    stop_watches(&s);
    s.bus
        .as_ref()
        .unwrap()
        .reply("quick-tunnel::release", Ok(Value::Null));
    s.cleanup().await.unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert!(data.repos.is_empty());
    assert!(data.watches["w1"].lease_id.is_none());
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn applied_post_with_lost_response_adopts_exact_url_and_patches_secret() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    configure_gh(
        dir.path(),
        json!({"fail_create":true,"create_applied":true}),
    );
    s.ensure_hook("owner/repo", "https://one.example", "one")
        .await
        .unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.repos["owner/repo"].hook_id, Some(42));
    assert!(data.repos["owner/repo"].error.is_none());
    assert_eq!(gh_calls(dir.path(), "POST").len(), 1);
    let patches = gh_calls(dir.path(), "PATCH");
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["body"]["config"]["secret"], "test-secret");
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn failed_listing_keeps_durable_url_and_recover_finds_applied_post_without_repost() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path()).await;
    tests::seed(&s);
    configure_gh(
        dir.path(),
        json!({"fail_create":true,"create_applied":true,"fail_list":true}),
    );
    assert!(s
        .ensure_hook("owner/repo", "https://old.example", "old")
        .await
        .is_err());
    let data = s.store().unwrap().read().unwrap();
    assert!(data.repos["owner/repo"].create_started);
    assert_eq!(
        data.repos["owner/repo"].url.as_deref(),
        Some("https://old.example/webhooks/github/endpoint")
    );
    assert!(data.repos["owner/repo"].hook_id.is_none());
    // Reopen only this temporary store to model process loss of in-memory state.
    reopen_store(&mut s, dir.path()).await;
    let persisted = s.store().unwrap().read().unwrap();
    assert_eq!(
        persisted.repos["owner/repo"].url,
        data.repos["owner/repo"].url
    );
    configure_gh(dir.path(), json!({"fail_list":false}));
    let bus = s.bus.as_ref().unwrap();
    for id in ["w1", "w2"] {
        bus.reply("quick-tunnel::acquire", Ok(json!({"lease_id":id})));
        bus.reply(
            "quick-tunnel::status",
            Ok(serde_json::to_value(tunnel("ready", Some("https://new.example"), "new")).unwrap()),
        );
    }
    s.recover(None).await.unwrap();
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.repos["owner/repo"].hook_id, Some(42));
    assert_eq!(
        data.repos["owner/repo"].url.as_deref(),
        Some("https://new.example/webhooks/github/endpoint")
    );
    assert_eq!(gh_calls(dir.path(), "POST").len(), 1);
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn paginated_listing_finds_hook_after_unrelated_first_page_and_cleanup_deletes_only_owned() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    crash_intent(&s);
    configure_gh(
        dir.path(),
        json!({
            "hook":{"id":42,"config":{"url":"https://old.example/webhooks/github/endpoint","secret":"masked"}},
            "hook_pages":[[{"id":99,"config":{"url":"https://foreign.example/webhooks/github/endpoint"}}], ["managed"]]
        }),
    );
    stop_watches(&s);
    s.cleanup().await.unwrap();
    assert!(s.store().unwrap().read().unwrap().repos.is_empty());
    let gets = gh_calls(dir.path(), "GET");
    assert_eq!(
        gets[1]["endpoint"],
        "repos/owner/repo/hooks?per_page=100&page=2"
    );
    let deletes = gh_calls(dir.path(), "DELETE");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0]["endpoint"], "repos/owner/repo/hooks/42");
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn incomplete_unsafe_and_duplicate_hook_listings_never_clear_or_mutate_intent() {
    for fields in [
        json!({"hook_pages":[[],[]],"fail_hook_page":2}),
        json!({"hook_pages":[[],[],[],[],[],[]]}),
        json!({"hook_next_override":"https://evil.example/repos/owner/repo/hooks?page=2"}),
        json!({"hook_next_override":"https://api.github.com/repos/other/repo/hooks?page=2"}),
        json!({"hook_pages":[[],[]],"hook_next_override":"https://api.github.com/repos/owner/repo/hooks?page=2"}),
        json!({"hook_pages":[[
            {"id":42,"config":{"url":"https://old.example/webhooks/github/endpoint"}},
            {"id":43,"config":{"url":"https://old.example/webhooks/github/endpoint"}}
        ]]}),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path()).await;
        tests::seed(&s);
        crash_intent(&s);
        configure_gh(dir.path(), fields);
        assert!(s
            .ensure_hook("owner/repo", "https://new.example", "new")
            .await
            .is_err());
        let data = s.store().unwrap().read().unwrap();
        assert!(data.repos["owner/repo"].create_started);
        assert!(data.repos["owner/repo"].hook_id.is_none());
        assert!(gh_calls(dir.path(), "POST").is_empty());
        assert!(gh_calls(dir.path(), "PATCH").is_empty());
        assert!(gh_calls(dir.path(), "GET").len() <= 5);
        s.iii.shutdown_async().await;
    }
}

#[tokio::test]
async fn patch_failure_keeps_owned_id_for_cleanup_but_not_hook_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    configure_gh(
        dir.path(),
        json!({"fail_create":true,"create_applied":true,"fail_patch":true}),
    );
    assert!(s
        .ensure_hook("owner/repo", "https://one.example", "one")
        .await
        .is_err());
    let data = s.store().unwrap().read().unwrap();
    assert_eq!(data.repos["owner/repo"].hook_id, Some(42));
    assert!(data.repos["owner/repo"].error.is_some());
    assert!(!s.status("w1").unwrap().health.hook_ready);
    stop_watches(&s);
    s.cleanup().await.unwrap();
    assert!(s.store().unwrap().read().unwrap().repos.is_empty());
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn failed_or_malformed_acquire_preserves_old_lease_and_continues_other_watch_and_redelivery()
{
    for reply in [Err(offline()), Ok(json!({"lease_id":""}))] {
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path()).await;
        tests::seed(&s);
        s.ensure_hook("owner/repo", "https://one.example", "one")
            .await
            .unwrap();
        configure_gh(
            dir.path(),
            json!({"delivery_pages":[[{"id":7,"guid":"retry","status_code":500}]]}),
        );
        s.store()
            .unwrap()
            .change(|d| {
                for (id, w) in &mut d.watches {
                    w.lease_id = Some(format!("old-{id}"));
                }
                Ok(())
            })
            .unwrap();
        let bus = s.bus.as_ref().unwrap();
        bus.reply("quick-tunnel::acquire", reply);
        bus.reply("quick-tunnel::acquire", Ok(json!({"lease_id":"new-w2"})));
        bus.reply(
            "quick-tunnel::status",
            Ok(serde_json::to_value(tunnel("starting", None, "new")).unwrap()),
        );
        assert!(s.recover(None).await.is_err());
        let data = s.store().unwrap().read().unwrap();
        assert_eq!(data.watches["w1"].lease_id.as_deref(), Some("old-w1"));
        assert_eq!(data.watches["w2"].lease_id.as_deref(), Some("new-w2"));
        assert!(data.last_error.is_some());
        assert!(gh_calls(dir.path(), "POST")
            .iter()
            .any(|c| c["endpoint"] == "repos/owner/repo/hooks/42/deliveries/7/attempts"));
        stop_watches(&s);
        bus.reply("quick-tunnel::release", Ok(Value::Null));
        bus.reply("quick-tunnel::release", Ok(Value::Null));
        s.cleanup().await.unwrap();
        let releases: Vec<_> = bus
            .calls()
            .into_iter()
            .filter(|c| c.function == "quick-tunnel::release")
            .collect();
        assert_eq!(releases[0].payload["lease_id"], "old-w1");
        assert_eq!(releases[1].payload["lease_id"], "new-w2");
        s.iii.shutdown_async().await;
    }
}

#[tokio::test]
async fn redelivery_skips_accepted_and_successful_guids_and_deduplicates_missing_guids_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "one")
        .await
        .unwrap();
    s.store()
        .unwrap()
        .change(|d| {
            d.deliveries.insert("42:accepted".into());
            d.deliveries.insert("99:unaccepted".into());
            Ok(())
        })
        .unwrap();
    configure_gh(
        dir.path(),
        json!({"calls":[],"delivery_pages":[[
            {"id":1,"guid":"accepted","status_code":500},
            {"id":2,"guid":"unaccepted","status_code":500},
            {"id":3,"guid":"eventually-ok","status_code":500},
            {"id":4,"status_code":500},
            {"id":5,"status_code":202}
        ],[
            {"id":6,"guid":"unaccepted","status_code":500},
            {"id":7,"guid":"eventually-ok","status_code":202,"redelivery":true},
            {"id":4,"status_code":500},
            {"id":8,"guid":"","status_code":500},
            {"id":8,"guid":null,"status_code":500}
        ]]}),
    );
    s.redeliver("owner/repo").await.unwrap();
    let endpoints: Vec<_> = gh_calls(dir.path(), "POST")
        .into_iter()
        .map(|c| c["endpoint"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        endpoints,
        [2, 4, 8].map(|id| format!("repos/owner/repo/hooks/42/deliveries/{id}/attempts"))
    );
    s.iii.shutdown_async().await;
}

#[tokio::test]
async fn redelivery_refreshes_durable_acceptance_while_fetching_later_pages() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path()).await;
    tests::seed(&s);
    s.ensure_hook("owner/repo", "https://one.example", "one")
        .await
        .unwrap();
    configure_gh(
        dir.path(),
        json!({"calls":[],"pause_delivery_page":2,"delivery_pages":[
            [{"id":1,"guid":"arrived","status_code":500}],
            [{"id":1,"guid":"arrived","status_code":500}]
        ]}),
    );
    let acceptance = async {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !dir.path().join("mock.waiting").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        s.store()
            .unwrap()
            .change(|d| {
                d.deliveries.insert("42:arrived".into());
                Ok(())
            })
            .unwrap();
        std::fs::write(dir.path().join("mock.continue"), "").unwrap();
    };
    let (result, ()) = tokio::join!(s.redeliver("owner/repo"), acceptance);
    result.unwrap();
    assert!(gh_calls(dir.path(), "POST").is_empty());
    s.iii.shutdown_async().await;
}
