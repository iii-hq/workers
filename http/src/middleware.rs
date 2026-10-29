//! Middleware execution: global (config-driven) and per-route (observer
//! args-driven) `preHandler` middleware.
//!
//! Ported from the in-engine `iii-http` `execute_middleware` /
//! `build_middleware_input` (`views.rs`), adapted to this worker's
//! invocation: instead of `engine.call` wrapped in an external
//! `tokio::time::timeout`, this worker uses `IIIClient::call`, which
//! already applies `timeout_ms` internally and surfaces an expired timeout as
//! `Error::Timeout`.
//!
//! A middleware that short-circuits produces a response as a function returns
//! one (`{ status_code, headers, body }`), which
//! [`crate::types::HttpResponse::from_function_return`] turns into the HTTP
//! response: the server does it for global middleware, and `http::on-request`
//! returns it for route middleware.

use std::collections::HashMap;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use serde_json::{json, Value};

/// Outcome of a middleware invocation that did not short-circuit the request.
pub enum MiddlewareOutcome {
    Continue,
}

/// Generates a short, time-based id correlating an error response with server
/// logs. Not cryptographically unique -- just enough to grep for.
pub(crate) fn generate_error_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", timestamp & 0xFFFF_FFFF_FFFF)
}

/// Standardized error envelope: `{"error": {"code", "message", "error_id"?}}`.
pub(crate) fn error_body(code: &str, message: &str, error_id: Option<&str>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(id) = error_id {
        error["error_id"] = json!(id);
    }
    json!({ "error": error })
}

/// A response as a function returns it, for [`crate::types::HttpResponse::from_function_return`].
pub(crate) fn response_value(status_code: u16, body: Value) -> Value {
    json!({ "status_code": status_code, "body": body })
}

/// Builds the 500 error envelope for a failed `IIIClient::call`,
/// propagating the invoked function's own error code/message when available
/// (mirrors the engine's `Error::Remote` -> `{code, message}` propagation, see
/// `engine/src/workers/rest_api/views.rs:248-251,518`). Falls back to
/// `"INTERNAL_ERROR"` + the error's `Display` for non-remote errors (timeouts,
/// transport/serde failures, etc).
pub(crate) fn error_body_for_call_error(err: &Error, error_id: Option<&str>) -> Value {
    match err.invocation_error() {
        Some(inv) => error_body(&inv.code, &inv.message, error_id),
        None => error_body("INTERNAL_ERROR", &err.to_string(), error_id),
    }
}

/// Builds the payload sent to a middleware function: the lifecycle phase, the
/// inbound request shape (path/query params, headers, method), and an empty
/// `context` map for the middleware to populate (unused by this worker today).
pub fn build_middleware_input(
    path_params: &HashMap<String, String>,
    query_params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    method: &str,
) -> Value {
    json!({
        "phase": "preHandler",
        "request": {
            "path_params": path_params,
            "query_params": query_params,
            "headers": headers,
            "method": method,
        },
        "context": {},
    })
}

/// [`build_middleware_input`] for a serialized [`crate::types::HttpRequest`]:
/// the input of a route's middleware, built in `http::on-request`'s deliver
/// hook from the call's input.
pub fn middleware_input_for(request: &Value) -> Value {
    let field = |name: &str| request.get(name).cloned().unwrap_or_else(|| json!({}));
    json!({
        "phase": "preHandler",
        "request": {
            "path_params": field("path_params"),
            "query_params": field("query_params"),
            "headers": field("headers"),
            "method": request.get("method").cloned().unwrap_or(Value::Null),
        },
        "context": {},
    })
}

/// Turns a middleware's `{"action": "respond", "response": {...}}` result into
/// the response it describes: `status_code` (default 200), `body` (default
/// null), and `headers` (an object of header values; entries that are not
/// valid header name/value pairs are skipped when the response is built).
fn respond_to_response(response: &Value) -> Value {
    let status = response
        .get("status_code")
        .and_then(|s| s.as_u64())
        .unwrap_or(200);
    let body = response.get("body").cloned().unwrap_or(Value::Null);
    let headers: HashMap<String, String> = response
        .get("headers")
        .and_then(|h| serde_json::from_value(h.clone()).ok())
        .unwrap_or_default();
    json!({ "status_code": status, "headers": headers, "body": body })
}

/// Invokes a middleware function and maps its result to either
/// [`MiddlewareOutcome::Continue`] or a short-circuit response (the `Err`
/// arm, as a function returns one): `action: "continue"` continues,
/// `action: "respond"` builds the response from
/// `response.{status_code,body,headers}`, any other/missing action logs and
/// continues, a call error maps to a 500 envelope, and a timed-out call maps
/// to a 504 `MIDDLEWARE_TIMEOUT` envelope.
pub async fn execute_middleware(
    iii: &IIIClient,
    fn_id: &str,
    input: Value,
    timeout_ms: u64,
) -> Result<MiddlewareOutcome, Value> {
    let result = iii
        .call(CallRequest {
            function_id: fn_id.to_string(),
            payload: input,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await;

    match result {
        Ok(value) => match value.get("action").and_then(|a| a.as_str()) {
            Some("continue") => Ok(MiddlewareOutcome::Continue),
            Some("respond") => {
                let response = value.get("response").cloned().unwrap_or(Value::Null);
                Err(respond_to_response(&response))
            }
            other => {
                tracing::warn!(
                    middleware_fn = %fn_id,
                    action = ?other,
                    "Middleware returned invalid action, treating as continue"
                );
                Ok(MiddlewareOutcome::Continue)
            }
        },
        Err(Error::Timeout) => {
            tracing::error!(middleware_fn = %fn_id, "Middleware timed out");
            Err(response_value(
                504,
                error_body("MIDDLEWARE_TIMEOUT", "Middleware timeout", None),
            ))
        }
        Err(err) => {
            let error_id = generate_error_id();
            tracing::error!(
                middleware_fn = %fn_id,
                error = %err,
                error_id = %error_id,
                "Middleware execution failed"
            );
            Err(response_value(
                500,
                error_body_for_call_error(&err, Some(&error_id)),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HttpResponse;
    use axum::http::StatusCode;

    /// The HTTP response a short-circuit value turns into.
    fn http(value: Value) -> axum::response::Response {
        HttpResponse::from_function_return(value).into_axum_response()
    }

    // =========================================================================
    // build_middleware_input
    // =========================================================================

    #[test]
    fn build_middleware_input_has_prehandler_phase_and_request_shape() {
        let mut headers = HashMap::new();
        headers.insert("x-trace".to_string(), "abc".to_string());
        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "42".to_string());
        let mut query_params = HashMap::new();
        query_params.insert("q".to_string(), "1".to_string());

        let input = build_middleware_input(&path_params, &query_params, &headers, "POST");

        assert_eq!(input["phase"], "preHandler");
        assert_eq!(input["request"]["method"], "POST");
        assert_eq!(input["request"]["path_params"]["id"], "42");
        assert_eq!(input["request"]["query_params"]["q"], "1");
        assert_eq!(input["request"]["headers"]["x-trace"], "abc");
        assert_eq!(input["context"], json!({}));
    }

    #[test]
    fn middleware_input_for_a_serialized_request_matches_the_server_input() {
        let mut headers = HashMap::new();
        headers.insert("x-trace".to_string(), "abc".to_string());
        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "42".to_string());
        let mut query_params = HashMap::new();
        query_params.insert("q".to_string(), "1".to_string());
        let request = json!({
            "path_params": path_params,
            "query_params": query_params,
            "headers": headers,
            "path": "/users/:id",
            "method": "POST",
            "body": { "ignored": true },
        });

        assert_eq!(
            middleware_input_for(&request),
            build_middleware_input(&path_params, &query_params, &headers, "POST")
        );
    }

    // =========================================================================
    // respond_to_response
    // =========================================================================

    #[tokio::test]
    async fn respond_to_response_defaults_status_200_and_null_body() {
        let resp = http(respond_to_response(&json!({})));
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"null");
    }

    #[tokio::test]
    async fn respond_to_response_honors_status_code_and_body() {
        let resp = http(respond_to_response(&json!({
            "status_code": 403,
            "body": { "denied": true },
        })));
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, json!({ "denied": true }));
    }

    #[tokio::test]
    async fn respond_to_response_applies_valid_headers_and_skips_invalid() {
        let resp = http(respond_to_response(&json!({
            "status_code": 200,
            "headers": { "X-Custom": "value", "": "skip-empty-name" },
        })));
        assert_eq!(
            resp.headers().get("x-custom").and_then(|v| v.to_str().ok()),
            Some("value")
        );
    }

    // =========================================================================
    // error_body
    // =========================================================================

    #[test]
    fn error_body_omits_error_id_when_none() {
        let body = error_body("MIDDLEWARE_TIMEOUT", "Middleware timeout", None);
        assert_eq!(body["error"]["code"], "MIDDLEWARE_TIMEOUT");
        assert!(body["error"].get("error_id").is_none());
    }

    #[test]
    fn error_body_includes_error_id_when_present() {
        let body = error_body("INTERNAL_ERROR", "boom", Some("abc123"));
        assert_eq!(body["error"]["error_id"], "abc123");
    }

    // =========================================================================
    // error_body_for_call_error
    // =========================================================================

    #[test]
    fn error_body_for_call_error_propagates_remote_code_and_message() {
        let err = Error::Remote {
            code: "CUSTOM".to_string(),
            message: "boom".to_string(),
            stacktrace: None,
        };
        let body = error_body_for_call_error(&err, Some("abc123"));
        assert_eq!(body["error"]["code"], "CUSTOM");
        assert_eq!(body["error"]["message"], "boom");
        assert_eq!(body["error"]["error_id"], "abc123");
    }

    #[test]
    fn error_body_for_call_error_falls_back_to_internal_error_for_non_remote() {
        let body = error_body_for_call_error(&Error::Timeout, None);
        assert_eq!(body["error"]["code"], "INTERNAL_ERROR");
        assert_eq!(body["error"]["message"], "invocation timed out");
        assert!(body["error"].get("error_id").is_none());
    }
}
