#![allow(dead_code)]
//! Test backend: an echo function bound to an `http` trigger.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use iii_http::types::HttpRequest;

/// Register `test.echo` (returns a 200 whose body echoes the request) and bind
/// it to an `http` trigger for `api_path` + `http_method`. The echoed body has
/// shape `{ method, path_params, query_params, headers, body }`, so
/// `HttpResponse::from_function_return` yields a 200 carrying it.
///
/// The function id embeds method+path so multiple routes can coexist in one
/// engine across tests in the same binary.
pub async fn register_echo_backend(iii: &Arc<IIIClient>, api_path: &str, http_method: &str) {
    let function_id = format!("test.echo {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": {
                    "method": req.method,
                    "path_params": req.path_params,
                    "query_params": req.query_params,
                    "headers": req.headers,
                    "body": req.body,
                }
            }))
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger");
}

/// Register `test.trace` bound to an `http` trigger; on every invocation the
/// function records the W3C trace id it runs under — i.e. the trace context the
/// http worker propagated onto the outbound trigger — into the returned slot.
///
/// The captured value is 32 lowercase hex chars; an all-zero id
/// (`00000000000000000000000000000000`) means the function ran under no/invalid
/// trace context. An outbound trace-propagation test compares this against the
/// worker's HTTP span trace id to prove `request -> worker -> function` is one
/// unified trace.
pub async fn register_trace_capturing_backend(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
) -> Arc<Mutex<Option<String>>> {
    let function_id = format!("test.trace {http_method} {api_path}");
    let slot = Arc::new(Mutex::new(None));
    let sink = slot.clone();

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let sink = sink.clone();
            async move {
                use opentelemetry::trace::TraceContextExt;
                let trace_id = opentelemetry::Context::current()
                    .span()
                    .span_context()
                    .trace_id();
                *sink.lock().unwrap() = Some(format!("{trace_id}"));
                Ok::<Value, Error>(json!({
                    "status_code": 200,
                    "body": { "method": req.method },
                }))
            }
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger");

    slot
}

/// Like [`register_echo_backend`], but returns the trigger handle so a test can
/// call [`Trigger::unregister`] to remove the route and assert the subsequent
/// 404. The trigger does NOT unregister on drop, so ignoring the returned
/// handle keeps the route registered.
pub async fn register_removable_echo_backend(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
) -> Trigger {
    let function_id = format!("test.echo {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": { "method": req.method, "path": req.path },
            }))
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger")
}

/// Register `test.echo` (same as [`register_echo_backend`]) bound to an
/// `http` trigger that also carries `middleware_function_ids`, so the
/// per-route middleware path runs before the handler. The function id embeds
/// method+path+a marker so it doesn't collide with [`register_echo_backend`]
/// routes registered elsewhere in the same test binary.
pub async fn register_echo_backend_with_middleware(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
    middleware_function_ids: &[&str],
) {
    let function_id = format!("test.echo.mw {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": {
                    "method": req.method,
                    "path_params": req.path_params,
                    "query_params": req.query_params,
                    "headers": req.headers,
                    "body": req.body,
                }
            }))
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({
            "api_path": api_path,
            "http_method": http_method,
            "middleware_function_ids": middleware_function_ids,
        }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger with middleware");
}

/// Register a middleware function that always returns `{"action": "continue"}`.
/// Returns a call counter the middleware increments on every invocation, so a
/// test can assert it actually ran (`== 1` after one request).
pub fn register_continue_middleware(iii: &Arc<IIIClient>, function_id: &str) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    iii.register_function(
        function_id.to_string(),
        RegisterFunction::new_async(move |_req: Value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<Value, Error>(json!({ "action": "continue" }))
            }
        }),
    );
    calls
}

/// Register a middleware function that always short-circuits with a 403
/// response carrying `{"denied": true}`.
/// Returns a call counter the middleware increments on every invocation, so a
/// test can assert how many requests it short-circuited.
pub fn register_respond_middleware(iii: &Arc<IIIClient>, function_id: &str) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    iii.register_function(
        function_id.to_string(),
        RegisterFunction::new_async(move |_req: Value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<Value, Error>(json!({
                    "action": "respond",
                    "response": {
                        "status_code": 403,
                        "body": { "denied": true },
                    },
                }))
            }
        }),
    );
    calls
}

/// Register `test.echo` (same as [`register_echo_backend`]) bound to an
/// `http` trigger that also carries `condition_function_id`, so the
/// conditional-execution path runs before the per-route middleware/handler.
/// The function id embeds method+path+a marker so it doesn't collide with
/// other routes registered elsewhere in the same test binary.
pub async fn register_echo_backend_with_condition(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
    condition_function_id: &str,
) {
    let function_id = format!("test.echo.cond {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": {
                    "method": req.method,
                    "path_params": req.path_params,
                    "query_params": req.query_params,
                    "headers": req.headers,
                    "body": req.body,
                }
            }))
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
        namespace: iii.namespace(),
    })
    .expect("register http trigger with condition");
}

/// Register a condition function that always returns `true` (so the gated
/// route proceeds to its handler).
pub fn register_condition_pass(iii: &Arc<IIIClient>, function_id: &str) {
    iii.register_function(
        function_id.to_string(),
        RegisterFunction::new_async(|_req: Value| async move { Ok::<Value, Error>(json!(true)) }),
    );
}

/// Register a condition function that always returns `false` (so the gated
/// route is skipped with `422 CONDITION_NOT_MET`).
pub fn register_condition_fail(iii: &Arc<IIIClient>, function_id: &str) {
    iii.register_function(
        function_id.to_string(),
        RegisterFunction::new_async(|_req: Value| async move { Ok::<Value, Error>(json!(false)) }),
    );
}

/// Register `test.slow` bound to an `http` trigger; the function sleeps for
/// `sleep_ms` before returning 200. Used by timeout-parity tests to force the
/// request past `default_timeout` and observe whether the tower
/// `TimeoutLayer` (504) or the handler's own invocation-timeout mapping (500)
/// wins the race.
pub async fn register_slow_backend(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
    sleep_ms: u64,
) {
    let function_id = format!("test.slow {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |_req: HttpRequest| async move {
            tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": { "slept_ms": sleep_ms },
            }))
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger");
}

/// Register `test.sleep` bound to an `http` trigger; the function sleeps for
/// `sleep_ms` then returns 200. Used by the concurrency-limit test: several
/// concurrent requests each sleep briefly so the `ConcurrencyLimitLayer` has a
/// chance to queue excess requests behind the configured permit count.
pub async fn register_sleep_backend(
    iii: &Arc<IIIClient>,
    api_path: &str,
    http_method: &str,
    sleep_ms: u64,
) {
    let function_id = format!("test.sleep {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |_req: HttpRequest| async move {
            tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": { "slept_ms": sleep_ms },
            }))
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger");
}

/// Register `test.error` bound to an `http` trigger; the function always
/// fails with a structured `Error::Remote { code: "CUSTOM_BOOM", .. }`. Used
/// to assert the worker's 500 envelope propagates the function's own
/// code/message instead of a hard-coded `INTERNAL_ERROR`.
pub async fn register_erroring_backend(iii: &Arc<IIIClient>, api_path: &str, http_method: &str) {
    let function_id = format!("test.error {http_method} {api_path}");

    iii.register_function(
        function_id.clone(),
        RegisterFunction::new_async(move |_req: HttpRequest| async move {
            Err::<Value, Error>(Error::Remote {
                code: "CUSTOM_BOOM".to_string(),
                message: "backend exploded".to_string(),
                stacktrace: None,
            })
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_http::TRIGGER_TYPE.to_string(),
        function_id,
        config: json!({ "api_path": api_path, "http_method": http_method }),
        metadata: None,
        namespace: iii.namespace(),
    })
    .expect("register http trigger");
}
