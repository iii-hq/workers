//! End-to-end CORS coverage against a running engine.
//!
//! Exercises `server::build_cors_layer` (`http/src/server.rs`) through real
//! HTTP requests: the permissive fallback (no `cors` config) on an actual
//! request, and the configured-origins/methods path on a preflight `OPTIONS`
//! request. Each test skips (early-returns) when no engine is reachable, and
//! tests are serialized so their trigger registrations don't race (mirrors
//! `e2e_methods.rs`).

mod common;

use common::{backend, engine, worker};
use serial_test::serial;

use iii_http::config::CorsConfig;

#[tokio::test]
#[serial]
async fn no_cors_config_is_permissive() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/cors-permissive", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/cors-permissive").await;

    let url = format!("http://{}/cors-permissive", boot.local_addr);
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Origin", "http://example.com")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let allow_origin = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("permissive CORS should set access-control-allow-origin")
        .to_str()
        .unwrap();
    // `CorsLayer::permissive()` allows any origin, reflected as `*`.
    assert_eq!(allow_origin, "*");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn configured_origin_and_method_are_allowed_on_preflight() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker_with_cors(
        iii.clone(),
        CorsConfig {
            allowed_origins: vec!["http://allowed.com".to_string()],
            allowed_methods: vec!["GET".to_string(), "POST".to_string()],
        },
    )
    .await;
    // Preflight is handled by the CORS layer itself, before the request
    // reaches the route table, so no backend route is needed for this path.

    let url = format!("http://{}/cors-configured", boot.local_addr);
    let resp = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &url)
        .header("Origin", "http://allowed.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "preflight should succeed, got {}",
        resp.status()
    );
    let allow_origin = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("allowed origin should be reflected")
        .to_str()
        .unwrap();
    assert_eq!(allow_origin, "http://allowed.com");

    let allow_methods = resp
        .headers()
        .get("access-control-allow-methods")
        .expect("configured methods should be present")
        .to_str()
        .unwrap();
    assert!(
        allow_methods.contains("POST"),
        "access-control-allow-methods should include POST, got {allow_methods}"
    );

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn disallowed_origin_is_not_reflected_on_preflight() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker_with_cors(
        iii.clone(),
        CorsConfig {
            allowed_origins: vec!["http://allowed.com".to_string()],
            allowed_methods: vec!["GET".to_string(), "POST".to_string()],
        },
    )
    .await;

    let url = format!("http://{}/cors-disallowed", boot.local_addr);
    let resp = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &url)
        .header("Origin", "http://notallowed.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();

    // The disallowed origin must not be reflected back: either the header is
    // absent, or it does not echo the disallowed origin.
    let allow_origin = resp.headers().get("access-control-allow-origin");
    match allow_origin {
        None => {}
        Some(value) => assert_ne!(value.to_str().unwrap(), "http://notallowed.com"),
    }

    boot.shutdown().await;
}
