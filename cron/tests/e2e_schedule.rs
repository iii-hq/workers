mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{Deliver, RegisterFunction, Source};
use serde_json::json;
use serial_test::serial;

use common::engine;

#[tokio::test]
#[serial]
async fn an_observing_target_runs_on_its_schedule_with_the_tick() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };

    let boot = iii_cron::boot::start(iii.clone());

    let fires = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(tokio::sync::Mutex::new(None::<serde_json::Value>));
    {
        let fires = fires.clone();
        let seen = seen.clone();
        iii.register_function(
            "e2e::cron-backend",
            RegisterFunction::new_async(move |payload: serde_json::Value| {
                let fires = fires.clone();
                let seen = seen.clone();
                async move {
                    fires.fetch_add(1, Ordering::SeqCst);
                    *seen.lock().await = Some(payload);
                    Ok::<_, iii_sdk::Error>(json!({"ok": true}))
                }
            }),
        );
    }

    let observer = iii.observe(
        Source::function("cron::on-tick")
            .args(json!({"expression": "*/1 * * * * *"}))
            .deliver(Deliver::Output),
        "e2e::cron-backend",
    );
    tokio::time::timeout(Duration::from_secs(10), observer.ready())
        .await
        .expect("observer settled")
        .expect("observer accepted");

    common::wait_for_fires(&fires, 1).await;
    let payload = seen.lock().await.clone().unwrap();
    assert_eq!(payload["expression"], "*/1 * * * * *");
    assert!(payload["scheduled_time"].as_str().is_some());
    assert!(payload["actual_time"].as_str().is_some());

    observer.dispose().await.unwrap();
    boot.shutdown().await;
}
