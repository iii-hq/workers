//! Redis adapter integration: the `WATCH`/`MULTI` compare-and-set path that
//! unit tests cannot reach without a server.
//!
//! Connect-or-skip like the engine e2e suite: unreachable Redis skips the
//! test (CI and redis-less boxes stay green); set `III_REDIS_REQUIRE` to any
//! value to fail loudly instead. Point at a server with `III_REDIS_URL`
//! (default `redis://localhost:6379`).

use std::sync::Arc;
use std::time::Duration;

use iii_helpers::stream::UpdateOp;
use iii_state::adapters::{CompareAndSetOutcome, StateAdapter, build_adapter};
use iii_state::config::StateConfig;
use serde_json::json;
use serial_test::serial;
use uuid::Uuid;

fn redis_url() -> String {
    std::env::var("III_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string())
}

async fn connect() -> Option<Arc<dyn StateAdapter>> {
    let url = redis_url();
    let config: StateConfig = serde_json::from_value(json!({
        "adapter": {"name": "redis", "config": {"redis_url": url}},
    }))
    .expect("redis state config");
    match build_adapter(&config).await {
        Ok(adapter) => Some(adapter),
        Err(error) => {
            if std::env::var("III_REDIS_REQUIRE").is_ok() {
                panic!("III_REDIS_REQUIRE is set but Redis is unreachable at {url}: {error}");
            }
            eprintln!(
                "[skip] no Redis reachable at {url} — skipping (set III_REDIS_REQUIRE=1 to fail \
                 instead)"
            );
            None
        }
    }
}

// Serial: the dropped-connection test kills every client connection.
#[tokio::test]
#[serial]
async fn redis_cas_swaps_only_on_match() {
    let Some(adapter) = connect().await else {
        return;
    };
    // Redis outlives the test process — a unique scope isolates reruns.
    let scope = format!("e2e-redis-cas-{}", Uuid::new_v4());

    // Set-if-absent claims the key and reports no previous value.
    let claimed = adapter
        .compare_and_set(&scope, "slot", None, json!({"owner": "a", "n": 1}))
        .await
        .expect("set-if-absent");
    assert_eq!(claimed, CompareAndSetOutcome::Swapped { old_value: None });

    // A second set-if-absent loses and returns the current value.
    let lost = adapter
        .compare_and_set(&scope, "slot", None, json!({"owner": "b"}))
        .await
        .expect("competing set-if-absent");
    assert_eq!(
        lost,
        CompareAndSetOutcome::NotSwapped {
            current: json!({"owner": "a", "n": 1})
        }
    );

    // Equality is parsed-JSON equality: object key order must not matter.
    let reordered = serde_json::from_str(r#"{"n":1,"owner":"a"}"#).unwrap();
    let swapped = adapter
        .compare_and_set(&scope, "slot", Some(&reordered), json!({"owner": "b"}))
        .await
        .expect("reordered-expectation CAS");
    assert!(
        matches!(swapped, CompareAndSetOutcome::Swapped { old_value: Some(v) } if v == json!({"owner": "a", "n": 1}))
    );

    // A stored null counts as absent — in both directions.
    adapter
        .set(&scope, "nulled", serde_json::Value::Null)
        .await
        .expect("seed a stored null");
    let over_null = adapter
        .compare_and_set(
            &scope,
            "nulled",
            Some(&serde_json::Value::Null),
            json!("filled"),
        )
        .await
        .expect("CAS with a null expectation");
    assert!(matches!(over_null, CompareAndSetOutcome::Swapped { .. }));

    // The redis adapter refuses barriers rather than faking atomicity.
    let cfg = iii_state::barrier::BarrierConfig {
        id: "join".into(),
        expect: iii_state::barrier::Expect::Count(2),
        key_from: None,
        carry: None,
    };
    let refusal = adapter
        .barrier_arrive(&scope, "join", &cfg, &json!({"key": "a"}))
        .await
        .expect_err("redis barriers must refuse, not race");
    assert!(refusal.to_string().contains("kv adapter"), "{refusal}");

    for key in ["slot", "nulled"] {
        adapter.delete(&scope, key).await.expect("cleanup");
    }
}

/// What a Redis restart (node drain, upgrade, OOM kill) does to the worker:
/// the server closes every client socket. Each command after that must
/// reconnect and answer, not fail once with "broken pipe".
#[tokio::test]
#[serial]
async fn redis_commands_survive_a_dropped_connection() {
    let Some(adapter) = connect().await else {
        return;
    };
    let scope = format!("e2e-redis-drop-{}", Uuid::new_v4());
    let admin = redis::Client::open(redis_url()).expect("redis url");
    let drop_connections = async || {
        let mut conn = admin
            .get_multiplexed_async_connection()
            .await
            .expect("admin connection");
        redis::cmd("CLIENT")
            .arg(&["KILL", "TYPE", "normal"])
            .query_async::<()>(&mut conn)
            .await
            .expect("CLIENT KILL");
        // Let the adapter's connection see the close, as it would while idle.
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    adapter.set(&scope, "k", json!("v1")).await.expect("seed");

    drop_connections().await;
    let got = adapter.get(&scope, "k").await.expect("get after drop");
    assert_eq!(got.as_deref(), Some(&json!("v1")));

    drop_connections().await;
    let set = adapter
        .set(&scope, "k", json!({"n": 1}))
        .await
        .expect("set after drop");
    assert_eq!(set.old_value, Some(json!("v1")));

    drop_connections().await;
    let updated = adapter
        .update(&scope, "k", vec![UpdateOp::set("m", json!(2))])
        .await
        .expect("update after drop");
    assert_eq!(updated.new_value, json!({"n": 1, "m": 2}));

    // Not repeatable: it must still work after a drop, and apply once.
    drop_connections().await;
    let counted = adapter
        .update(&scope, "c", vec![UpdateOp::increment("n", 1)])
        .await
        .expect("increment after drop");
    assert_eq!(counted.new_value, json!({"n": 1}));
    adapter.delete(&scope, "c").await.expect("cleanup");

    drop_connections().await;
    let swapped = adapter
        .compare_and_set(&scope, "k", Some(&json!({"n": 1, "m": 2})), json!("v3"))
        .await
        .expect("compare-and-set after drop");
    assert!(matches!(swapped, CompareAndSetOutcome::Swapped { .. }));

    drop_connections().await;
    assert_eq!(
        adapter.list(&scope).await.expect("list after drop").len(),
        1
    );

    drop_connections().await;
    let keys = adapter
        .list_keys(&scope)
        .await
        .expect("list_keys after drop");
    assert_eq!(keys, vec!["k".to_string()]);

    drop_connections().await;
    let groups = adapter.list_groups().await.expect("list_groups after drop");
    assert!(groups.contains(&scope));

    drop_connections().await;
    adapter
        .delete(&scope, "k")
        .await
        .expect("delete after drop");
    assert_eq!(adapter.get(&scope, "k").await.expect("get"), None);
}
