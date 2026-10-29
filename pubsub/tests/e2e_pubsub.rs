mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Deliver, RegisterFunction, Source};
use serde_json::json;
use serial_test::serial;

use common::engine;

#[tokio::test]
#[serial]
async fn publish_reaches_the_observers_of_its_topic() {
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

    let mut observers = Vec::new();
    for fn_id in ["e2e::pubsub-listener-a", "e2e::pubsub-listener-b"] {
        let observer = iii.observe(
            Source::function(iii_pubsub::PUBLISH_FUNCTION_ID)
                .filters(json!({"input.topic": {"$eq": "e2e.orders"}}))
                .deliver(Deliver::Input),
            fn_id,
        );
        tokio::time::timeout(Duration::from_secs(5), observer.ready())
            .await
            .expect("observer never became active")
            .expect("observing publish");
        observers.push(observer);
    }

    // Call `publish` over the bus — the EXACT path external callers use.
    let result = iii
        .call(CallRequest {
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

    // Another topic reaches neither observer.
    iii.call(CallRequest {
        function_id: "publish".to_string(),
        payload: json!({"topic": "e2e.invoices", "data": {"id": 7}}),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
    .expect("publish to another topic should succeed");

    common::wait_for_deliveries(&deliveries, 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(deliveries.load(Ordering::SeqCst), 2, "one delivery each");
    let seen = seen.lock().await;
    for payload in seen.iter() {
        // `deliver: input` hands each observer the call's input.
        assert_eq!(payload["topic"], json!("e2e.orders"));
        assert_eq!(payload["data"], json!({"id": 42}));
    }

    for observer in observers {
        observer.dispose().await.expect("dispose observer");
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
        .call(CallRequest {
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
