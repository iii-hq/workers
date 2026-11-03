//! Live check against a real engine: a provider client registers the
//! security-scan change functions and calls them through the production
//! feed, a separate consumer client observes one with a filter, and the test
//! checks filtered delivery, args refusal and disposal end to end.
//!
//! ```sh
//! SECURITY_SCAN_LIVE_III_URL=ws://127.0.0.1:49641 cargo test --test live_change_feed -- --ignored
//! ```

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, Deliver, IIIClient, InitOptions, RegisterFunction, Source};
use security_scan::events::{
    self, ChangeFeed, ChangeKind, ReconciliationChangedEventV1, ON_RECONCILIATION_CHANGED_FN,
    ON_RUN_CHANGED_FN,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

fn client(url: &str, name: &str) -> Arc<IIIClient> {
    Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: "0.0.0".into(),
                name: name.into(),
                os: std::env::consts::OS.into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ))
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

fn event(run_id: &str, repository: &str) -> ReconciliationChangedEventV1 {
    ReconciliationChangedEventV1 {
        run_id: run_id.into(),
        repository: repository.into(),
    }
}

#[tokio::test]
#[ignore = "needs a live engine: set SECURITY_SCAN_LIVE_III_URL"]
async fn change_functions_work_on_a_live_engine() {
    let url = std::env::var("SECURITY_SCAN_LIVE_III_URL")
        .expect("set SECURITY_SCAN_LIVE_III_URL to a live engine");

    let provider = client(&url, "security-scan-live-provider");
    events::register_functions(&provider);
    let feed = ChangeFeed::new(provider.clone());

    let consumer = client(&url, "security-scan-live-consumer");
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    consumer.register_function(
        "security-scan-live::on-reconciliation",
        RegisterFunction::new(move |payload: Value| -> Result<Value, iii_sdk::Error> {
            let _ = tx.send(payload);
            Ok(Value::Null)
        }),
    );
    let observer = consumer.observe(
        Source::function(ON_RECONCILIATION_CHANGED_FN)
            .filters(json!({ "input.repository": { "$eq": "iii" } }))
            .deliver(Deliver::Output),
        "security-scan-live::on-reconciliation",
    );
    tokio::time::timeout(Duration::from_secs(10), observer.ready())
        .await
        .expect("observer active within 10 s")
        .expect("observer accepted");
    wait_for("the provider to see the observer", || {
        provider.is_observed(ON_RECONCILIATION_CHANGED_FN)
    })
    .await;
    println!("observer active through the engine");

    // The change functions take no args: the engine refuses them.
    let refused = consumer.observe(
        Source::function(ON_RUN_CHANGED_FN).args(json!({ "repository": "iii" })),
        "security-scan-live::on-reconciliation",
    );
    let error = tokio::time::timeout(Duration::from_secs(10), refused.ready())
        .await
        .expect("an answer within 10 s")
        .expect_err("args are refused");
    assert!(error.to_string().contains("INVALID_ARGS"), "{error}");
    assert!(!provider.is_observed(ON_RUN_CHANGED_FN));
    println!("args on a no-args change function refused: {error}");

    // Filtered out: a change in another repository is called but not delivered.
    assert!(
        feed.emit(ChangeKind::Reconciliation, &event("sec_other", "other"))
            .await
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err(),
        "the filter keeps other repositories out"
    );
    assert!(
        feed.emit(ChangeKind::Reconciliation, &event("sec_live", "iii"))
            .await
    );
    let received = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("delivery within 10 s")
        .expect("consumer channel open");
    println!("consumer received {received}");
    assert_eq!(
        received,
        json!({ "run_id": "sec_live", "repository": "iii" })
    );

    observer.dispose().await.expect("dispose the observer");
    wait_for("the provider to see the observer go", || {
        !provider.is_observed(ON_RECONCILIATION_CHANGED_FN)
    })
    .await;
    println!("observer disposed through the engine");
    assert!(
        !feed
            .emit(ChangeKind::Reconciliation, &event("sec_live", "iii"))
            .await
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err(),
        "no delivery after dispose"
    );

    consumer.shutdown_async().await;
    provider.shutdown_async().await;
}
