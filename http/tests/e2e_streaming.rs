//! End-to-end streaming coverage against a running engine: a function that
//! writes control + body frames to the response channel produces a chunked
//! HTTP response, while a plain value-returning function still yields a
//! buffered response.
//!
//! Mirrors the harness used by `e2e_routing.rs` -- see that file's module doc
//! for the connect-or-skip / per-test-worker rationale.

mod common;

use std::sync::Arc;

use common::{backend, engine, worker};
use iii_sdk::channel::ChannelWriter;
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};
use serial_test::serial;

use iii_http::types::HttpRequest;

/// Register a function that streams its response: it opens a writer on the
/// response channel ref, sends a `set_status` control message, writes two body
/// chunks, closes the channel, and returns `null` (no buffered body). Bound to
/// an `http` trigger for `api_path` + `http_method`.
async fn register_stream_backend(iii: &Arc<IIIClient>, api_path: &str, http_method: &str) {
    let function_id = format!("test.stream {http_method} {api_path}");
    let ws_url = engine::ws_url();

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let ws_url = ws_url.clone();
            async move {
                let writer = ChannelWriter::new(&ws_url, &req.response);
                writer
                    .send_message(&json!({ "type": "set_status", "status_code": 201 }).to_string())
                    .await?;
                writer.write(b"chunk-1").await?;
                writer.write(b"chunk-2").await?;
                writer.close().await?;
                Ok::<Value, Error>(Value::Null)
            }
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
    })
    .expect("register http trigger for stream backend");
}

#[tokio::test]
#[serial]
async fn streamed_response_chunks_and_status() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    register_stream_backend(&iii, "/stream", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/stream").await;

    let url = format!("http://{}/stream", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 201);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "chunk-1chunk-2");

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn buffered_response_still_works() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/buffered", "POST").await;
    common::wait_for_route(&boot.routes, "POST", "/buffered").await;

    let url = format!("http://{}/buffered", boot.local_addr);
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({ "hello": "world" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "POST");
    assert_eq!(v["body"]["hello"], "world");

    boot.shutdown().await;
}
