//! Redis adapter integration: the `WATCH`/`MULTI` compare-and-set path that
//! unit tests cannot reach without a server.
//!
//! Connect-or-skip like the engine e2e suite: unreachable Redis skips the
//! test (CI and redis-less boxes stay green); set `III_REDIS_REQUIRE` to any
//! value to fail loudly instead. Point at a server with `III_REDIS_URL`
//! (default `redis://localhost:6379`).

use std::sync::Arc;

use iii_state::adapters::{CompareAndSetOutcome, StateAdapter, build_adapter};
use iii_state::config::StateConfig;
use serde_json::json;
use uuid::Uuid;

async fn connect() -> Option<Arc<dyn StateAdapter>> {
    let url =
        std::env::var("III_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
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

#[tokio::test]
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
