//! Live check of the `counter_provider` example's `demo::on-counter-changed`
//! rules against a real engine: `onObserverAdded` rejects a counter outside
//! 1-64 bytes with `INVALID_ARGS` and a 65th observer with `OBSERVER_LIMIT`.
//!
//! ```bash
//! cargo run --example counter_provider -- ws://127.0.0.1:49671 &
//! A2UI_LIVE_URL=ws://127.0.0.1:49671 cargo test --test live_counter_provider -- --ignored --nocapture
//! ```
//!
//! Its own test binary, so no other live test observes the function while it
//! fills all 64 slots.

use std::time::Duration;

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, Error, IIIClient, InitOptions, ObserverHandle,
    RegisterFunction, Source,
};
use serde_json::{json, Value};

const ON_COUNTER_CHANGED_FN: &str = "demo::on-counter-changed";
const SINK_FN: &str = "a2ui-live-limits::sink";
const MAX_OBSERVERS: usize = 64;

async fn observer_count(iii: &IIIClient) -> u64 {
    iii.call(CallRequest {
        function_id: "demo::observers::count".into(),
        payload: json!({}),
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .unwrap()["count"]
        .as_u64()
        .unwrap()
}

async fn wait_for_count(iii: &IIIClient, expected: u64) {
    for _ in 0..100 {
        if observer_count(iii).await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("provider observer count never reached {expected}");
}

fn observe(iii: &IIIClient, counter: &str) -> ObserverHandle {
    iii.observe(
        Source::function(ON_COUNTER_CHANGED_FN)
            .args(json!({ "counter": counter }))
            .deliver(Deliver::Output),
        SINK_FN,
    )
}

/// The code and message an observer's `ready()` ends with.
async fn rejection(handle: &ObserverHandle) -> (String, String) {
    match tokio::time::timeout(Duration::from_secs(5), handle.ready())
        .await
        .expect("observer settles in time")
    {
        Err(Error::Remote { code, message, .. }) => (code, message),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine and the counter_provider example; set A2UI_LIVE_URL"]
async fn counter_observers_are_validated_and_capped() {
    let url = std::env::var("A2UI_LIVE_URL").expect("A2UI_LIVE_URL");
    let iii = register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                name: "a2ui-live-limits".into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    iii.register_function(
        SINK_FN,
        RegisterFunction::new(|event: Value| Ok::<Value, Error>(event))
            .description("Live check: target of the counter observers."),
    );
    let mut registered = false;
    for _ in 0..50 {
        let info = iii
            .call(CallRequest {
                function_id: "engine::functions::info".into(),
                payload: json!({ "function_id": ON_COUNTER_CHANGED_FN }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;
        if info.is_ok() {
            registered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(registered, "start the counter_provider example first");
    wait_for_count(&iii, 0).await;

    // INVALID_ARGS: the provider's own check, past the args_format.
    for counter in [String::new(), "x".repeat(65)] {
        let handle = observe(&iii, &counter);
        let (code, message) = rejection(&handle).await;
        println!("counter of {} bytes -> {code}: {message}", counter.len());
        assert_eq!(code, "INVALID_ARGS");
        assert!(message.contains("counter must be 1-64 bytes"), "{message}");
        let _ = handle.dispose().await;
    }
    let longest = observe(&iii, &"x".repeat(64));
    tokio::time::timeout(Duration::from_secs(5), longest.ready())
        .await
        .expect("a 64-byte counter is accepted in time")
        .unwrap();
    longest.dispose().await.unwrap();
    wait_for_count(&iii, 0).await;

    // OBSERVER_LIMIT: 64 observers are accepted, the 65th is not.
    let mut accepted = Vec::with_capacity(MAX_OBSERVERS);
    for index in 0..MAX_OBSERVERS {
        let handle = observe(&iii, &format!("c{index}"));
        tokio::time::timeout(Duration::from_secs(5), handle.ready())
            .await
            .expect("observer accepted in time")
            .unwrap();
        accepted.push(handle);
    }
    wait_for_count(&iii, MAX_OBSERVERS as u64).await;
    let extra = observe(&iii, "c64");
    let (code, message) = rejection(&extra).await;
    println!("observer {} -> {code}: {message}", MAX_OBSERVERS + 1);
    assert_eq!(code, "OBSERVER_LIMIT");
    assert!(message.contains("at most 64 observers"), "{message}");
    let _ = extra.dispose().await;

    // A freed slot is taken again.
    accepted.pop().unwrap().dispose().await.unwrap();
    wait_for_count(&iii, MAX_OBSERVERS as u64 - 1).await;
    let again = observe(&iii, "c63");
    tokio::time::timeout(Duration::from_secs(5), again.ready())
        .await
        .expect("observer accepted in time")
        .unwrap();
    accepted.push(again);

    for handle in accepted {
        handle.dispose().await.unwrap();
    }
    wait_for_count(&iii, 0).await;
    iii.shutdown_async().await;
}
