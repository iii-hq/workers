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

/// Poll the live config cell until a `cors` policy appears (the observable
/// effect of the `configuration:updated` propagation for CORS), or panic.
async fn wait_for_cors(config: &configuration::ConfigCell) {
    for _ in 0..100 {
        if config.read().await.cors.is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("config reload (cors) never propagated to the cell");
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
    configuration::register_config_trigger(&iii, boot.config.clone(), boot.router.clone())
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

/// CORS hot-reload via the swappable [`iii_http::server::HotRouter`]: boot
/// permissive (default → any origin), flip to a restricted `cors` policy via
/// `configuration::set`, and prove the running listener now enforces it WITHOUT
/// a restart (same `boot.local_addr`). Restricting CORS is a baked tower layer,
/// not a per-request cell read, so a passing assertion here can only be
/// explained by the router's layers being rebuilt and swapped live.
#[tokio::test]
#[serial]
async fn cors_restricted_via_config_set_applies_without_restart() {
    // A dedicated client: this test registers `http::on-config-change`, the same
    // id the middleware test registers, and one client cannot register an id
    // twice. Own connection → own registration namespace.
    let iii = engine::connect_fresh().await;

    // Boot with the default (permissive) CORS: any origin allowed.
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/cors-reload", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/cors-reload").await;

    configuration::register_config(&iii, None)
        .await
        .expect("register http configuration schema");
    configuration::register_config_trigger(&iii, boot.config.clone(), boot.router.clone())
        .expect("bind configuration trigger");

    let url = format!("http://{}/cors-reload", boot.local_addr);
    let client = reqwest::Client::new();

    // Before reload: permissive CORS reflects `*` for any origin.
    let resp = client
        .get(&url)
        .header("Origin", "http://anything.example")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*"),
        "expected permissive `*` CORS before the reload",
    );

    // Flip to a RESTRICTED CORS policy (same address → live layer rebuild).
    let new_value = json!({
        "host": "127.0.0.1",
        "port": 0,
        "cors": { "allowed_origins": ["http://allowed.com"], "allowed_methods": [] }
    });
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".to_string(),
        payload: json!({ "id": configuration::CONFIG_ID, "value": new_value }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .expect("configuration::set");

    // Wait for the `configuration:updated` event to swap the cell (and, via the
    // same-address branch, rebuild the router's CORS layer).
    wait_for_cors(&boot.config).await;

    // After reload: an allowed origin is reflected verbatim (no longer `*`).
    let resp = client
        .get(&url)
        .header("Origin", "http://allowed.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://allowed.com"),
        "expected the restricted origin reflected after the reload (proves live swap)",
    );

    // A disallowed origin must NOT get the permissive `*` (the restricted layer
    // omits the header for unmatched origins).
    let resp = client
        .get(&url)
        .header("Origin", "http://evil.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_ne!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*"),
        "disallowed origin must not receive permissive `*` after the reload",
    );

    boot.shutdown().await;
    iii.shutdown_async().await;
}
