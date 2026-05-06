//! E2E: write a credential, kill auth-credentials worker, restart, read it back.
//!
//! Requires:
//! - `IIITEST_ENGINE_URL`: WebSocket URL of a running iii engine
//! - `IIITEST_WORKER_BIN`: absolute path to the iii-auth-credentials binary
//!
//! Marked `#[ignore]` so cargo test default runs skip it. To execute:
//!     cargo build --release -p iii-auth-credentials
//!     IIITEST_ENGINE_URL=ws://127.0.0.1:49134 \
//!       IIITEST_WORKER_BIN=$(pwd)/target/release/iii-auth-credentials \
//!       cargo test -p iii-auth-credentials --test restart_e2e -- --ignored

use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn spawn_worker(engine_url: &str, bin: &str) -> Child {
    Command::new(bin)
        .env("III_URL", engine_url)
        // Default backend for the worker is iii-state. We force it explicitly
        // here so the test is robust against future default flips.
        .env("AUTH_CREDENTIALS_STORE", "iii_state")
        // Suppress worker output so it doesn't interleave with cargo test output.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn iii-auth-credentials")
}

async fn wait_for_ready() {
    // Crude: give the worker time to register on the bus. A real harness
    // would poll a status function with backoff.
    tokio::time::sleep(Duration::from_secs(2)).await;
}

#[tokio::test]
#[ignore = "requires live engine + IIITEST_WORKER_BIN; opt in via --ignored"]
async fn credential_survives_worker_restart() {
    let url = std::env::var("IIITEST_ENGINE_URL").expect("set IIITEST_ENGINE_URL");
    let bin = std::env::var("IIITEST_WORKER_BIN").expect("set IIITEST_WORKER_BIN");

    let provider = format!("e2e-restart-{}", std::process::id());
    let api_key = format!("sk-e2e-{}", std::process::id());

    // 1. Spawn worker, wait for ready, write a credential.
    let mut worker = spawn_worker(&url, &bin);
    wait_for_ready().await;

    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    iii.trigger(iii_sdk::TriggerRequest {
        function_id: "auth::set_token".into(),
        // Credential serializes per the `#[serde(tag = "type", rename_all =
        // "snake_case")]` attribute on the `Credential` enum (auth-credentials
        // src/lib.rs lines 17-34): `ApiKey { key }` => `{ "type": "api_key",
        // "key": "..." }`.
        payload: serde_json::json!({
            "provider": &provider,
            "credential": { "type": "api_key", "key": &api_key },
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .expect("auth::set_token failed");

    // 2. Kill the worker.
    worker.kill().expect("kill worker");
    worker.wait().expect("wait for worker exit");

    // 3. Restart and wait.
    let mut worker = spawn_worker(&url, &bin);
    wait_for_ready().await;

    // 4. Read back; assert credential survived.
    let resp = iii
        .trigger(iii_sdk::TriggerRequest {
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

    // 5. Cleanup. Best-effort — the actual assertion above is what matters.
    let _ = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: "auth::delete_token".into(),
            payload: serde_json::json!({ "provider": &provider }),
            action: None,
            timeout_ms: None,
        })
        .await;
    worker.kill().ok();
    let _ = worker.wait();
}
