//! End-to-end middleware coverage (per-route) against a running engine.
//!
//! Mirrors `e2e_methods.rs`'s harness usage: skip (early-return) when no
//! engine is reachable, boot one worker per test, bind backend triggers, wait
//! for the route to land, then issue the request.

mod common;

use common::{backend, engine, worker};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn per_route_middleware_continue_runs_handler() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;

    let mw_calls = backend::register_continue_middleware(&iii, "test.mw.continue");
    backend::register_echo_backend_with_middleware(
        &iii,
        "/mw-continue",
        "GET",
        &["test.mw.continue"],
    )
    .await;
    common::wait_for_route(&boot.routes, "GET", "/mw-continue").await;

    let url = format!("http://{}/mw-continue?q=1", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "GET");
    assert_eq!(v["query_params"]["q"], "1");

    // Prove the middleware actually ran (exactly once for the one request).
    assert_eq!(
        mw_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "continue middleware should have been invoked exactly once"
    );

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn per_route_middleware_respond_short_circuits_before_handler() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;

    backend::register_respond_middleware(&iii, "test.mw.respond");
    backend::register_echo_backend_with_middleware(
        &iii,
        "/mw-respond",
        "GET",
        &["test.mw.respond"],
    )
    .await;
    common::wait_for_route(&boot.routes, "GET", "/mw-respond").await;

    let url = format!("http://{}/mw-respond", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 403);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v, serde_json::json!({ "denied": true }));

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn global_middleware_respond_short_circuits_every_route() {
    let iii = engine::get_or_init().await;

    let mw_calls = backend::register_respond_middleware(&iii, "test.mw.global_respond");
    let boot =
        worker::start_http_worker_with_global_middleware(iii.clone(), &["test.mw.global_respond"])
            .await;

    // Two different routes: global middleware must fire on each.
    backend::register_echo_backend(&iii, "/mw-global-a", "GET").await;
    backend::register_echo_backend(&iii, "/mw-global-b", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/mw-global-a").await;
    common::wait_for_route(&boot.routes, "GET", "/mw-global-b").await;

    let client = reqwest::Client::new();
    for path in ["/mw-global-a", "/mw-global-b"] {
        let url = format!("http://{}{path}", boot.local_addr);
        let resp = client.get(&url).send().await.unwrap();
        assert_eq!(resp.status(), 403, "global middleware should short-circuit {path}");
        let v: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(v, serde_json::json!({ "denied": true }));
    }

    // Global middleware ran on both routes (2 requests -> 2 invocations).
    assert_eq!(
        mw_calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "global middleware should have run once per request across both routes"
    );

    boot.shutdown().await;
}
