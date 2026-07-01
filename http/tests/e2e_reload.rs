//! End-to-end hot-reload coverage against a running engine + configuration
//! worker.
//!
//! Skips (early-return) when no engine is reachable. Boots one worker with a
//! permissive config (no middleware), wires the `configuration:updated` reload
//! trigger, then flips a hot field — the global `middleware` chain — via
//! `configuration::set` and asserts the running server picks it up WITHOUT a
//! restart: requests return 200 before the reload and 403 after.

mod common;

use std::time::Duration;

use common::{backend, engine, worker};
use iii_http::configuration;
use iii_sdk::protocol::TriggerRequest;
use serde_json::json;
use serial_test::serial;

/// Poll the live config cell until a global middleware entry appears (the
/// observable effect of the `configuration:updated` propagation), or panic.
async fn wait_for_middleware(config: &configuration::ConfigCell) {
    for _ in 0..100 {
        if !config.read().await.middleware.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("config reload (global middleware) never propagated to the cell");
}

#[tokio::test]
#[serial]
async fn global_middleware_added_via_config_set_blocks_without_restart() {
    let iii = engine::get_or_init().await;

    // Boot permissive (no middleware), then register the route + a middleware
    // function that short-circuits with 403.
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_respond_middleware(&iii, "test.block");
    backend::register_echo_backend(&iii, "/reload", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/reload").await;

    // Register the schema (required before `configuration::set`) and wire the
    // reload trigger to this worker's live config cell.
    configuration::register_config(&iii, None)
        .await
        .expect("register http configuration schema");
    configuration::register_config_trigger(&iii, boot.config.clone())
        .expect("bind configuration trigger");

    let url = format!("http://{}/reload", boot.local_addr);
    let client = reqwest::Client::new();

    // Before reload: no global middleware → handler runs → 200.
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "expected 200 before the config reload");

    // Flip a hot field: set a config whose global middleware blocks with 403.
    let new_value = json!({
        "host": "127.0.0.1",
        "port": 0,
        "middleware": [
            { "function_id": "test.block", "phase": "preHandler", "priority": 0 }
        ]
    });
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".to_string(),
        payload: json!({ "id": configuration::CONFIG_ID, "value": new_value }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .expect("configuration::set");

    // Wait for the `configuration:updated` event to flow through the reload
    // trigger and swap the cell.
    wait_for_middleware(&boot.config).await;

    // After reload: global middleware short-circuits every route → 403.
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 403, "expected 403 after the config reload");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, json!({ "denied": true }));

    boot.shutdown().await;
}
