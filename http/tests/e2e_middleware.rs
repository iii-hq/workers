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
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;

    backend::register_continue_middleware(&iii, "test.mw.continue");
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

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn per_route_middleware_respond_short_circuits_before_handler() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
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
    let Some(iii) = engine::get_or_init().await else {
        return;
    };

    backend::register_respond_middleware(&iii, "test.mw.global_respond");
    let boot =
        worker::start_http_worker_with_global_middleware(iii.clone(), &["test.mw.global_respond"])
            .await;

    backend::register_echo_backend(&iii, "/mw-global", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/mw-global").await;

    let url = format!("http://{}/mw-global", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 403);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v, serde_json::json!({ "denied": true }));

    boot.shutdown().await;
}
