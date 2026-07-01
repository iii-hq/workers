//! End-to-end routing coverage against a running engine: path params, query
//! params, header passthrough, and the 404 envelope.
//!
//! Mirrors the harness used by `e2e_methods.rs` -- see that file's module doc
//! for the connect-or-skip / per-test-worker rationale.

mod common;

use common::{backend, engine, worker};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn single_path_param_is_extracted() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/users/:id", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/users/:id").await;

    let url = format!("http://{}/users/42", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["path_params"]["id"], "42");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn multiple_path_params_are_extracted() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/a/:x/b/:y", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/a/:x/b/:y").await;

    let url = format!("http://{}/a/1/b/2", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["path_params"]["x"], "1");
    assert_eq!(v["path_params"]["y"], "2");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn query_params_are_parsed() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-query", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/echo-query").await;

    let url = format!("http://{}/echo-query?q=hello&n=2", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["query_params"]["q"], "hello");
    assert_eq!(v["query_params"]["n"], "2");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn custom_header_passes_through() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-headers", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/echo-headers").await;

    let url = format!("http://{}/echo-headers", boot.local_addr);
    let resp = reqwest::Client::new()
        .get(&url)
        .header("x-custom", "present")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["headers"]["x-custom"], "present");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn unregistered_route_returns_404_not_found() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;

    let url = format!("http://{}/no-such-route-here", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "NOT_FOUND");

    boot.shutdown().await;
}
