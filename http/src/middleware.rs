//! Middleware execution: global (config-driven) and per-route (trigger
//! config-driven) `preHandler` middleware.
//!
//! Ported from the in-engine `iii-http` `execute_middleware` /
//! `build_middleware_input` (`views.rs`), adapted to this worker's
//! invocation: instead of `engine.call` wrapped in an external
//! `tokio::time::timeout`, this worker uses `IIIClient::trigger`, which
//! already applies `timeout_ms` internally and surfaces an expired timeout as
//! `Error::Timeout`.

use std::collections::HashMap;

use axum::{
    Json,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use serde_json::{Value, json};

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

fn serialize_headers(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
}

/// Builds the payload sent to a middleware function: the lifecycle phase, the
/// inbound request shape (path/query params, headers, method), and an empty
/// `context` map for the middleware to populate (unused by this worker today).
pub fn build_middleware_input(
    path_params: &HashMap<String, String>,
    query_params: &HashMap<String, String>,
    headers: &HeaderMap,
    method: &str,
) -> Value {
    json!({
        "phase": "preHandler",
        "request": {
            "path_params": path_params,
            "query_params": query_params,
            "headers": serialize_headers(headers),
            "method": method,
        },
        "context": {},
    })
}

/// Turns a middleware's `{"action": "respond", "response": {...}}` result into
/// the axum response it describes: `status_code` (default 200), `body`
/// (default null), and `headers` (best-effort -- entries that don't parse as
/// valid header name/value pairs are skipped).
fn respond_to_response(response: &Value) -> Response {
    let status = response
        .get("status_code")
        .and_then(|s| s.as_u64())
        .unwrap_or(200) as u16;
    let body = response.get("body").cloned().unwrap_or(Value::Null);
    let headers_map: HashMap<String, String> = response
        .get("headers")
        .and_then(|h| serde_json::from_value(h.clone()).ok())
        .unwrap_or_default();

    let mut resp = (
        StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        Json(body),
    )
        .into_response();
    for (k, v) in headers_map {
        if let (Ok(name), Ok(val)) = (k.parse::<HeaderName>(), v.parse::<HeaderValue>()) {
            resp.headers_mut().insert(name, val);
        }
    }
    resp
}

/// Invokes a middleware function and maps its result to either
/// [`MiddlewareOutcome::Continue`] or a short-circuit [`Response`] (the `Err`
/// arm): `action: "continue"` continues, `action: "respond"` builds the
/// response from `response.{status_code,body,headers}`, any other/missing
/// action logs and continues, a call error maps to a 500 envelope, and a
/// timed-out call maps to a 504 `MIDDLEWARE_TIMEOUT` envelope.
pub async fn execute_middleware(
    iii: &IIIClient,
    fn_id: &str,
    input: Value,
    timeout_ms: u64,
) -> Result<MiddlewareOutcome, Response> {
    let result = iii
        .trigger(TriggerRequest {
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
            Err((
                StatusCode::GATEWAY_TIMEOUT,
                Json(error_body("MIDDLEWARE_TIMEOUT", "Middleware timeout", None)),
            )
                .into_response())
        }
        Err(err) => {
            let error_id = generate_error_id();
            tracing::error!(
                middleware_fn = %fn_id,
                error = %err,
                error_id = %error_id,
                "Middleware execution failed"
            );
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_body("INTERNAL_ERROR", &err.to_string(), Some(&error_id))),
            )
                .into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

    // =========================================================================
    // build_middleware_input
    // =========================================================================

    #[test]
    fn build_middleware_input_has_prehandler_phase_and_request_shape() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-trace"),
            HeaderValue::from_static("abc"),
        );
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

    // =========================================================================
    // respond_to_response
    // =========================================================================

    #[tokio::test]
    async fn respond_to_response_defaults_status_200_and_null_body() {
        let resp = respond_to_response(&json!({}));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn respond_to_response_honors_status_code_and_body() {
        let resp = respond_to_response(&json!({
            "status_code": 403,
            "body": { "denied": true },
        }));
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, json!({ "denied": true }));
    }

    #[tokio::test]
    async fn respond_to_response_applies_valid_headers_and_skips_invalid() {
        let resp = respond_to_response(&json!({
            "status_code": 200,
            "headers": { "X-Custom": "value", "": "skip-empty-name" },
        }));
        assert_eq!(
            resp.headers()
                .get("x-custom")
                .and_then(|v| v.to_str().ok()),
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
}
