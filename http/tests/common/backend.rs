//! Test backend: an echo function bound to an `http` trigger.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};

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
    })
    .expect("register http trigger");
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
    })
    .expect("register http trigger with middleware");
}

/// Register a middleware function that always returns `{"action": "continue"}`.
pub fn register_continue_middleware(iii: &Arc<IIIClient>, function_id: &str) {
    iii.register_function(
        function_id.to_string(),
        RegisterFunction::new_async(|_req: Value| async move {
            Ok::<Value, Error>(json!({ "action": "continue" }))
        }),
    );
}

/// Register a middleware function that always short-circuits with a 403
/// response carrying `{"denied": true}`.
pub fn register_respond_middleware(iii: &Arc<IIIClient>, function_id: &str) {
    iii.register_function(
        function_id.to_string(),
        RegisterFunction::new_async(|_req: Value| async move {
            Ok::<Value, Error>(json!({
                "action": "respond",
                "response": {
                    "status_code": 403,
                    "body": { "denied": true },
                },
            }))
        }),
    );
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
    })
    .expect("register http trigger");
}
