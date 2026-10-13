//! Live check against a real engine (run it without the deprecated stream
//! worker): a provider client registers the security-scan change trigger
//! types with the production deliverer, a separate consumer client binds a
//! function, and the test checks filtered delivery, filter rejection and
//! unregistration end to end.
//!
//! ```sh
//! SECURITY_SCAN_LIVE_III_URL=ws://127.0.0.1:49641 cargo test --test live_change_feed -- --ignored
//! ```

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use security_scan::events::{
    self, ChangeFeed, ChangeKind, IiiChangeDeliverer, ReconciliationChangedEventV1,
    RECONCILIATION_CHANGED, RUN_CHANGED,
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
async fn change_trigger_types_work_on_a_live_engine() {
    let url = std::env::var("SECURITY_SCAN_LIVE_III_URL")
        .expect("set SECURITY_SCAN_LIVE_III_URL to a live engine");

    let provider = client(&url, "security-scan-live-provider");
    let feed = Arc::new(ChangeFeed::new(Arc::new(IiiChangeDeliverer::new(
        provider.clone(),
    ))));
    events::register_trigger_types(&provider, &feed);

    let consumer = client(&url, "security-scan-live-consumer");
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    consumer.register_function(
        "security-scan-live::on-reconciliation",
        RegisterFunction::new(move |payload: Value| -> Result<Value, iii_sdk::Error> {
            let _ = tx.send(payload);
            Ok(Value::Null)
        }),
    );
    let binding = consumer
        .register_trigger(RegisterTriggerInput {
            trigger_type: RECONCILIATION_CHANGED.into(),
            function_id: "security-scan-live::on-reconciliation".into(),
            config: json!({ "repository": "iii" }),
            metadata: None,
            namespace: None,
            trigger_namespace: None,
        })
        .expect("bind reconciliation-changed");
    wait_for("the binding to reach the provider", || {
        feed.binding_count(ChangeKind::Reconciliation) == 1
    })
    .await;
    println!("binding registered through the engine");

    // A malformed filter is rejected by the provider and never stored.
    let _rejected = consumer
        .register_trigger(RegisterTriggerInput {
            trigger_type: RUN_CHANGED.into(),
            function_id: "security-scan-live::on-reconciliation".into(),
            config: json!({ "stream_name": "security-scan:runs" }),
            metadata: None,
            namespace: None,
            trigger_namespace: None,
        })
        .expect("send the malformed binding");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(feed.binding_count(ChangeKind::Run), 0);
    println!("malformed run-changed filter rejected by the provider");

    // Filtered out: a change in another repository is not delivered.
    assert_eq!(
        feed.emit(
            ChangeKind::Reconciliation,
            "other",
            "sec_other",
            &event("sec_other", "other"),
        )
        .await,
        0
    );
    assert_eq!(
        feed.emit(
            ChangeKind::Reconciliation,
            "iii",
            "sec_live",
            &event("sec_live", "iii"),
        )
        .await,
        1
    );
    let received = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("delivery within 10 s")
        .expect("consumer channel open");
    println!("consumer received {received}");
    // The engine may add routing fields (`_caller_worker_id`); the domain
    // fields are exactly the notification.
    assert_eq!(received["run_id"], "sec_live");
    assert_eq!(received["repository"], "iii");

    binding.unregister();
    wait_for("the unregister callback", || {
        feed.binding_count(ChangeKind::Reconciliation) == 0
    })
    .await;
    println!("binding unregistered through the engine");
    assert_eq!(
        feed.emit(
            ChangeKind::Reconciliation,
            "iii",
            "sec_live",
            &event("sec_live", "iii"),
        )
        .await,
        0
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err(),
        "no delivery after unregister"
    );

    events::unregister_trigger_types(&provider);
    consumer.shutdown_async().await;
    provider.shutdown_async().await;
}
