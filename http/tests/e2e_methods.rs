//! End-to-end HTTP method coverage against a running engine.
//!
//! Each test skips (early-returns) when no engine is reachable. They share one
//! engine connection (per-binary `OnceCell`) and are serialized so their
//! trigger registrations don't race. Each test boots its own worker first
//! (registering the `http` trigger type), then binds its backend trigger, so
//! the binding is delivered live to that worker's handler; `wait_for_route`
//! then waits for the route to land before issuing the request.

mod common;

use common::{backend, engine, worker};
use serde_json::json;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn get_echoes_method_and_query() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-get", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/echo-get").await;

    let url = format!("http://{}/echo-get?q=1", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "GET");
    assert_eq!(v["query_params"]["q"], "1");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn post_echoes_body() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-post", "POST").await;
    common::wait_for_route(&boot.routes, "POST", "/echo-post").await;

    let url = format!("http://{}/echo-post", boot.local_addr);
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({ "hi": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "POST");
    assert_eq!(v["body"]["hi"], 1);

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn put_echoes_body() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-put", "PUT").await;
    common::wait_for_route(&boot.routes, "PUT", "/echo-put").await;

    let url = format!("http://{}/echo-put", boot.local_addr);
    let resp = reqwest::Client::new()
        .put(&url)
        .json(&json!({ "n": 7 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "PUT");
    assert_eq!(v["body"]["n"], 7);

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn patch_echoes_body() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-patch", "PATCH").await;
    common::wait_for_route(&boot.routes, "PATCH", "/echo-patch").await;

    let url = format!("http://{}/echo-patch", boot.local_addr);
    let resp = reqwest::Client::new()
        .patch(&url)
        .json(&json!({ "p": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "PATCH");
    assert_eq!(v["body"]["p"], true);

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn delete_echoes_method() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/echo-delete", "DELETE").await;
    common::wait_for_route(&boot.routes, "DELETE", "/echo-delete").await;

    let url = format!("http://{}/echo-delete", boot.local_addr);
    let resp = reqwest::Client::new().delete(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "DELETE");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn unmatched_route_returns_404_envelope() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;

    let url = format!("http://{}/no-such-route", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "NOT_FOUND");

    boot.shutdown().await;
}
