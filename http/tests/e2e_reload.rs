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
    configuration::register_config_trigger(
        &iii,
        boot.config.clone(),
        boot.hot_router.clone(),
        boot.control.clone(),
        boot.apply_lock.clone(),
    )
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
    configuration::register_config_trigger(
        &iii,
        boot.config.clone(),
        boot.hot_router.clone(),
        boot.control.clone(),
        boot.apply_lock.clone(),
    )
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

/// Grab two distinct free loopback TCP ports by binding `127.0.0.1:0` twice and
/// reading the OS-assigned ports, then dropping the listeners so the ports are
/// free for the worker to (re)bind. A tiny TOCTOU race exists (another process
/// could grab a port before the worker does), but it is negligible in the test
/// environment and the alternative — ephemeral ports — cannot prove a *specific*
/// address moved.
fn two_free_ports() -> (u16, u16) {
    use std::net::TcpListener as StdListener;
    let l1 = StdListener::bind("127.0.0.1:0").unwrap();
    let l2 = StdListener::bind("127.0.0.1:0").unwrap();
    let p1 = l1.local_addr().unwrap().port();
    let p2 = l2.local_addr().unwrap().port();
    assert_ne!(p1, p2, "expected two distinct free ports");
    drop(l1);
    drop(l2);
    (p1, p2)
}

/// Poll until a GET to `url` returns 200, using a FRESH client each attempt so
/// no pooled keep-alive connection masks a moved listener. Returns true on
/// success within the deadline.
async fn wait_until_serves(url: &str) -> bool {
    for _ in 0..100 {
        let client = reqwest::Client::new();
        if let Ok(resp) = client.get(url).send().await {
            if resp.status() == 200 {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Poll until a NEW connection to `url` is refused/errors (the old listener is
/// fully closed). A fresh client per attempt avoids reusing a draining pooled
/// connection. Returns true if the address stops serving within the deadline.
async fn wait_until_refused(url: &str) -> bool {
    for _ in 0..50 {
        let client = reqwest::Client::new();
        if client.get(url).send().await.is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Poll the live config cell until its bound port equals `port` (the observable
/// effect of the rebind's config swap), or panic.
async fn wait_for_port(config: &configuration::ConfigCell, port: u16) {
    for _ in 0..100 {
        if config.read().await.port == port {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("config reload (port change) never propagated to the cell");
}

/// Host/port rebind (Phase B): boot on port P1, register a route, then flip the
/// config `port` to P2 via `configuration::set`. Prove the listener MOVED — P2
/// now serves the route and P1 stops accepting new connections — without a
/// restart (same `BootHandle`).
#[tokio::test]
#[serial]
async fn host_port_change_rebinds_listener_without_restart() {
    // Dedicated client: this test registers `http::on-config-change`, the same
    // id the other reload tests register (one id per client only).
    let iii = engine::connect_fresh().await;

    let (p1, p2) = two_free_ports();

    // Boot on the first fixed port and register a route.
    let boot = worker::start_http_worker_on(iii.clone(), "127.0.0.1", p1).await;
    backend::register_echo_backend(&iii, "/rebind", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/rebind").await;

    configuration::register_config(&iii, None)
        .await
        .expect("register http configuration schema");
    configuration::register_config_trigger(
        &iii,
        boot.config.clone(),
        boot.hot_router.clone(),
        boot.control.clone(),
        boot.apply_lock.clone(),
    )
    .expect("bind configuration trigger");

    let url_p1 = format!("http://127.0.0.1:{p1}/rebind");
    let url_p2 = format!("http://127.0.0.1:{p2}/rebind");

    // Before the rebind: P1 serves the route.
    assert!(
        wait_until_serves(&url_p1).await,
        "expected P1 ({p1}) to serve before the rebind"
    );

    // Flip the port to P2 (same host) → triggers a live rebind.
    let new_value = json!({ "host": "127.0.0.1", "port": p2 });
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".to_string(),
        payload: json!({ "id": configuration::CONFIG_ID, "value": new_value }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .expect("configuration::set");

    // Wait for the config swap to propagate, then for the new listener to serve.
    wait_for_port(&boot.config, p2).await;
    assert!(
        wait_until_serves(&url_p2).await,
        "expected P2 ({p2}) to serve the route after the rebind"
    );

    // The current address reported by the control cell must be the new port.
    assert_eq!(
        boot.current_addr().await.map(|a| a.port()),
        Some(p2),
        "control cell should report the rebound address"
    );

    // The old address must stop accepting new connections (graceful drain +
    // hard-abort safety net free the old port).
    assert!(
        wait_until_refused(&url_p1).await,
        "expected P1 ({p1}) to stop serving after the rebind"
    );

    boot.shutdown().await;
    iii.shutdown_async().await;
}
