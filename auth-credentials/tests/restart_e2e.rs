//! E2E: write a credential, kill auth-credentials worker, restart, read it back.
//!
//! Requires:
//! - `IIITEST_ENGINE_URL`: WebSocket URL of a running iii engine
//! - `IIITEST_WORKER_BIN`: absolute path to the auth-credentials binary
//!
//! Marked `#[ignore]` so cargo test default runs skip it. To execute:
//!     cargo build --release -p auth-credentials
//!     IIITEST_ENGINE_URL=ws://127.0.0.1:49134 \
//!       IIITEST_WORKER_BIN=$(pwd)/target/release/auth-credentials \
//!       cargo test -p auth-credentials --test restart_e2e -- --ignored

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::TriggerRequest;
use serde_json::json;

fn spawn_worker(engine_url: &str, bin: &str) -> Child {
    Command::new(bin)
        .env("III_URL", engine_url)
        .env("AUTH_CREDENTIALS_STORE", "iii_state")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn auth-credentials")
}

async fn wait_for_ready(iii: &iii_sdk::III) {
    let mut last_err = None;
    for _ in 0..40 {
        match iii
            .trigger(TriggerRequest {
                function_id: "auth::list_providers".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await
        {
            Ok(_) => return,
            Err(err) => last_err = Some(err.to_string()),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("auth-credentials did not register auth::* in time: {last_err:?}");
}

#[tokio::test]
#[ignore = "requires live engine + IIITEST_WORKER_BIN; opt in via --ignored"]
async fn credential_survives_worker_restart() {
    let url = std::env::var("IIITEST_ENGINE_URL").expect("set IIITEST_ENGINE_URL");
    let bin = std::env::var("IIITEST_WORKER_BIN").expect("set IIITEST_WORKER_BIN");

    let provider = format!("e2e-restart-{}", std::process::id());
    let api_key = format!("sk-e2e-{}", std::process::id());

    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());

    let mut worker = spawn_worker(&url, &bin);
    wait_for_ready(&iii).await;

    iii.trigger(TriggerRequest {
        function_id: "auth::set_token".into(),
        payload: serde_json::json!({
            "provider": &provider,
            "credential": { "type": "api_key", "key": &api_key },
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .expect("auth::set_token failed");

    worker.kill().expect("kill worker");
    worker.wait().expect("wait for worker exit");

    let mut worker = spawn_worker(&url, &bin);
    wait_for_ready(&iii).await;

    let resp = iii
        .trigger(TriggerRequest {
            function_id: "auth::get_token".into(),
            payload: serde_json::json!({ "provider": &provider }),
            action: None,
            timeout_ms: None,
        })
        .await
        .expect("auth::get_token failed");

    assert!(
        !resp.is_null(),
        "credential should survive restart; got null response",
    );
    let key_field = resp
        .get("key")
        .and_then(serde_json::Value::as_str)
        .expect("response missing `key` field");
    assert_eq!(key_field, api_key, "credential.key must round-trip");

    let _ = iii
        .trigger(TriggerRequest {
            function_id: "auth::delete_token".into(),
            payload: serde_json::json!({ "provider": &provider }),
            action: None,
            timeout_ms: None,
        })
        .await;
    worker.kill().ok();
    let _ = worker.wait();
}
