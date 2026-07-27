mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serial_test::serial;

use common::engine;

#[tokio::test]
#[serial]
async fn cron_trigger_fires_bound_function_with_parity_payload() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };

    let config = iii_cron::config::CronConfig::default();
    let boot = iii_cron::boot::start(iii.clone(), config)
        .await
        .expect("cron worker should boot");

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
                    Ok::<_, iii_sdk::Error>(serde_json::json!({"ok": true}))
                }
            }),
        );
    }

    let trigger = iii
        .register_trigger(RegisterTriggerInput {
            trigger_type: "cron".to_string(),
            function_id: "e2e::cron-backend".to_string(),
            config: serde_json::json!({"expression": "*/1 * * * * *"}),
            metadata: None,
            namespace: iii.namespace(),
        })
        .expect("trigger registration");

    common::wait_for_fires(&fires, 1).await;
    let payload = seen.lock().await.clone().unwrap();
    assert_eq!(payload["trigger"], "cron");
    assert!(payload["job_id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(payload["scheduled_time"].as_str().is_some());
    assert!(payload["actual_time"].as_str().is_some());

    trigger.unregister();
    boot.shutdown().await;
}
