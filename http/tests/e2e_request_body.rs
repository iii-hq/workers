//! End-to-end coverage for the request-body streaming direction: the worker
//! WRITES the incoming HTTP request body into a channel and hands the
//! function a `read`-direction ref (`HttpRequest::request_body`); the
//! function opens a [`ChannelReader`] on it and reads the body back.
//!
//! Mirrors `e2e_streaming.rs` (which covers the opposite, response-writing
//! direction) -- see that file's module doc for the connect-or-skip /
//! per-test-worker harness rationale.
//!
//! Per `http/src/handler.rs::dynamic_handler`: the worker always reads the
//! full body into memory first (`axum::body::to_bytes`), then -- regardless
//! of content-type -- spawns a detached task that writes those bytes as a
//! single Binary frame into the request-body channel and closes the writer,
//! so `ChannelReader::read_all` terminates once the write completes. There is
//! no separate JSON/non-JSON code path for the request-body channel itself
//! (JSON only affects the buffered `HttpRequest::body` field).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{engine, worker};
use iii_sdk::channel::ChannelReader;
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use serial_test::serial;

use iii_http::types::HttpRequest;

/// Bound wait for the reader: if the worker never closes the request-body
/// writer, `read_all` would hang forever and the test would never fail --
/// bound it so a real worker regression surfaces as a test failure instead of
/// a hang.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Register a function that reads the full request body off its
/// `request_body` channel (rather than the buffered `body` field) and echoes
/// it back so the test can assert the streamed bytes match exactly what was
/// sent. Bound to an `http` trigger for `api_path` + `http_method`.
async fn register_request_body_echo_backend(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
) {
    let function_id = format!("test.request_body_echo {http_method} {api_path}");
    let ws_url = engine::ws_url();

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let ws_url = ws_url.clone();
            async move {
                let reader = ChannelReader::new(&ws_url, &req.request_body);
                let bytes = tokio::time::timeout(READ_TIMEOUT, reader.read_all())
                    .await
                    .map_err(|_| Error::Remote {
                        code: "REQUEST_BODY_READ_TIMEOUT".to_string(),
                        message: "timed out reading request_body channel".to_string(),
                        stacktrace: None,
                    })?
                    .map_err(|e| Error::Remote {
                        code: "REQUEST_BODY_READ_ERROR".to_string(),
                        message: e.to_string(),
                        stacktrace: None,
                    })?;
                let received = String::from_utf8_lossy(&bytes).into_owned();
                Ok::<Value, Error>(json!({
                    "status_code": 200,
                    "body": { "received": received, "len": bytes.len() },
                }))
            }
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
    })
    .expect("register http trigger for request-body echo backend");
}

#[tokio::test]
#[serial]
async fn request_body_streams_to_function_for_non_json_content_type() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    register_request_body_echo_backend(&iii, "/upload", "POST").await;
    common::wait_for_route(&boot.routes, "POST", "/upload").await;

    let body = "streamed upload body";
    let url = format!("http://{}/upload", boot.local_addr);
    let resp = reqwest::Client::new()
        .post(&url)
        .header("content-type", "text/plain")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["received"], body);
    assert_eq!(v["len"], body.len());

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn request_body_streams_to_function_for_json_content_type() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    register_request_body_echo_backend(&iii, "/upload-json", "POST").await;
    common::wait_for_route(&boot.routes, "POST", "/upload-json").await;

    let payload = json!({ "hello": "world" });
    let expected_raw = serde_json::to_string(&payload).unwrap();
    let url = format!("http://{}/upload-json", boot.local_addr);
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["received"], expected_raw);
    assert_eq!(v["len"], expected_raw.len());

    boot.shutdown().await;
}
