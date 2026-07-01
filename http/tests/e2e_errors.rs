//! End-to-end coverage for the invoke-error 500 path: the worker must
//! propagate the invoked function's own structured error `code`/`message`
//! (an `Error::Remote`), not a hard-coded `INTERNAL_ERROR`.
//!
//! Mirrors the harness used by `e2e_routing.rs` -- see that file's module doc
//! for the connect-or-skip / per-test-worker rationale.

mod common;

use common::{backend, engine, worker};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn function_error_code_and_message_propagate_on_500() {
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_erroring_backend(&iii, "/boom", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/boom").await;

    let url = format!("http://{}/boom", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 500);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "CUSTOM_BOOM");
    assert_eq!(v["error"]["message"], "backend exploded");
    assert!(v["error"]["error_id"].is_string());

    boot.shutdown().await;
}
