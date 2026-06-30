//! End-to-end conditional execution (`condition_function_id`) coverage
//! against a running engine.
//!
//! Mirrors `e2e_middleware.rs`'s harness usage: skip (early-return) when no
//! engine is reachable, boot one worker per test, bind backend triggers, wait
//! for the route to land, then issue the request.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{backend, engine, worker};
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};
use serial_test::serial;

use iii_http::types::HttpRequest;

/// Registers `test.echo.cond <method> <path>` (mirrors
/// [`backend::register_echo_backend_with_condition`]) but also increments
/// `hits` on every invocation, so a test can assert the handler was (or was
/// not) actually called -- not just infer it from the response shape.
async fn register_counting_backend_with_condition(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
    condition_function_id: &str,
    hits: Arc<AtomicUsize>,
) {
    let function_id = format!("test.echo.cond.counted {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Ok::<Value, Error>(json!({
                    "status_code": 200,
                    "body": {
                        "method": req.method,
                        "body": req.body,
                    }
                }))
            }
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({
            "api_path": api_path,
            "http_method": http_method,
            "condition_function_id": condition_function_id,
        }),
        metadata: None,
    })
    .expect("register http trigger with condition");
}

#[tokio::test]
#[serial]
async fn condition_true_runs_handler() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;

    backend::register_condition_pass(&iii, "test.cond.pass");
    backend::register_echo_backend_with_condition(&iii, "/cond-pass", "GET", "test.cond.pass")
        .await;
    common::wait_for_route(&boot.routes, "GET", "/cond-pass").await;

    let url = format!("http://{}/cond-pass", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "GET");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn condition_false_returns_422_and_skips_handler() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;

    let hits = Arc::new(AtomicUsize::new(0));
    backend::register_condition_fail(&iii, "test.cond.fail");
    register_counting_backend_with_condition(
        &iii,
        "/cond-fail",
        "GET",
        "test.cond.fail",
        hits.clone(),
    )
    .await;
    common::wait_for_route(&boot.routes, "GET", "/cond-fail").await;

    let url = format!("http://{}/cond-fail", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 422);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "CONDITION_NOT_MET");
    assert_eq!(v["skipped"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 0, "handler must not run when condition is false");

    boot.shutdown().await;
}
