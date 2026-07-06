mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::RegisterFunction;
use serde_json::json;
use serial_test::serial;

use common::engine;

#[tokio::test]
#[serial]
async fn publish_fans_out_raw_data_to_subscribe_triggers() {
    let Some(iii) = engine::get_or_init().await else {
        return; // skip: engine absent
    };

    let boot = iii_pubsub::boot::start(iii.clone(), iii_pubsub::config::PubSubConfig::default())
        .await
        .expect("pubsub worker should boot");

    let deliveries = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    for fn_id in ["e2e::pubsub-listener-a", "e2e::pubsub-listener-b"] {
        let deliveries = deliveries.clone();
        let seen = seen.clone();
        iii.register_function(
            fn_id,
            RegisterFunction::new_async(move |payload: serde_json::Value| {
                let deliveries = deliveries.clone();
                let seen = seen.clone();
                async move {
                    deliveries.fetch_add(1, Ordering::SeqCst);
                    seen.lock().await.push(payload);
                    Ok::<_, Error>(json!(null))
                }
            }),
        );
    }

    for fn_id in ["e2e::pubsub-listener-a", "e2e::pubsub-listener-b"] {
        iii.register_trigger(RegisterTriggerInput {
            trigger_type: "subscribe".to_string(),
            function_id: fn_id.to_string(),
            config: json!({"topic": "e2e.orders"}),
            metadata: None,
        })
        .expect("trigger registration");
    }

    // Give the engine a beat to route the trigger bindings to this worker.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Call `publish` over the bus — the EXACT path external callers use today.
    let result = iii
        .trigger(TriggerRequest {
            function_id: "publish".to_string(),
            payload: json!({"topic": "e2e.orders", "data": {"id": 42}}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .expect("publish should succeed");
    assert!(
        result.is_null(),
        "publish returns a null result (builtin parity)"
    );

    common::wait_for_deliveries(&deliveries, 2).await;
    let seen = seen.lock().await;
    for payload in seen.iter() {
        // Parity: the subscriber receives the RAW published data, no envelope.
        // The engine additionally stamps cross-worker invocation payloads with
        // caller metadata (e.g. `_caller_worker_id`) — identical for the
        // builtin's `engine.call` — so assert the published field is present
        // rather than exact-equal.
        assert_eq!(payload.get("id"), Some(&json!(42)));
    }

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn publish_with_empty_topic_fails_with_topic_not_set() {
    // Own client (connect_fresh) so booting the worker here doesn't collide
    // with the first test's `publish` registration on the shared client.
    let Some(iii) = engine::connect_fresh().await else {
        return; // skip: engine absent
    };

    let boot = iii_pubsub::boot::start(iii.clone(), iii_pubsub::config::PubSubConfig::default())
        .await
        .expect("pubsub worker should boot");

    let err = iii
        .trigger(TriggerRequest {
            function_id: "publish".to_string(),
            payload: json!({"topic": "", "data": {}}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .expect_err("empty topic must fail");
    assert!(err.to_string().contains("topic_not_set"), "got: {err}");

    boot.shutdown().await;
    iii.shutdown_async().await;
}
