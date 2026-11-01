//! HTTP request/response types, route args, and control messages exchanged
//! between the worker and the function it invokes.

use std::collections::HashMap;

use axum::{
    body::Body,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use iii_sdk::channel::StreamChannelRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One HTTP request as the server received it: the input of
/// `http::on-request` and its output (the handler returns its input). An
/// observer's filters read it as `input`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InboundRequest {
    pub method: String,
    /// The request's path, e.g. `/orders/42`.
    pub path: String,
    pub query_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    /// The body parsed as JSON; `null` when it is empty, not JSON, or over
    /// the server's body limit.
    pub body: Value,
}

/// What the target of the matched route's observer receives: the request,
/// with the route's template as `path`, the values of its `:param` segments,
/// and the request-body and response channels.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpRequest {
    pub query_params: HashMap<String, String>,
    /// Values of the matched route's `:param` segments.
    pub path_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    /// The matched route's path template, e.g. `/orders/:id`.
    pub path: String,
    pub method: String,
    /// The body parsed as JSON; `null` when it is empty or not JSON.
    pub body: Value,

    /// The exact request bytes, for the target to read.
    pub request_body: StreamChannelRef,
    /// A streamed response, for the target to write.
    pub response: StreamChannelRef,
}

/// Response built from a function's return value, eventually turned into an
/// axum HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl HttpResponse {
    pub fn from_function_return(value: Value) -> Self {
        let status_code = value
            .get("status_code")
            .and_then(|v| v.as_u64())
            .unwrap_or(200) as u16;
        let headers = match value.get("headers") {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(parse_header_line)
                .collect(),
            Some(Value::Object(map)) => map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect(),
            _ => Vec::new(),
        };
        let body = value.get("body").cloned().unwrap_or(json!({}));
        HttpResponse {
            status_code,
            headers,
            body,
        }
    }

    /// Returns the value of the first header matching `name` case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Build an axum response, honoring user-set headers and choosing a body
    /// serialization based on Content-Type (string/bytes for non-JSON types,
    /// JSON otherwise).
    pub fn into_axum_response(self) -> Response {
        let status = StatusCode::from_u16(self.status_code).unwrap_or(StatusCode::OK);
        let user_content_type = self.header("content-type").map(|s| s.to_string());

        let (body_bytes, default_content_type) = match (&user_content_type, &self.body) {
            (Some(_), Value::String(s)) => (s.clone().into_bytes(), None),
            (Some(_), Value::Null) => (Vec::new(), None),
            (Some(ct), other) if !ct.to_ascii_lowercase().contains("application/json") => {
                (other.to_string().into_bytes(), None)
            }
            (_, body) => (
                serde_json::to_vec(body).unwrap_or_else(|_| b"null".to_vec()),
                Some("application/json"),
            ),
        };

        let mut builder = axum::http::Response::builder().status(status);
        let mut content_type_set = false;
        for (k, v) in &self.headers {
            match (
                HeaderName::try_from(k.as_str()),
                HeaderValue::try_from(v.as_str()),
            ) {
                (Ok(name), Ok(value)) => {
                    if name.as_str().eq_ignore_ascii_case("content-type") {
                        content_type_set = true;
                    }
                    builder = builder.header(name, value);
                }
                _ => {
                    tracing::warn!(header_name = %k, "Skipping invalid response header");
                }
            }
        }
        if !content_type_set {
            if let Some(ct) = default_content_type {
                builder = builder.header("content-type", ct);
            }
        }

        builder
            .body(Body::from(body_bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

fn parse_header_line(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), value.trim().to_string()))
}

/// Normalizes OpenAPI-style whole-segment path parameters (`/tasks/{tid}`) to
/// the internal colon format (`/tasks/:tid`).
pub fn normalize_api_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') && segment.len() > 2 {
                let inner = &segment[1..segment.len() - 1];
                if !inner.contains('{') && !inner.contains('}') {
                    return format!(":{inner}");
                }
            }
            segment.to_string()
        })
        .collect::<Vec<String>>()
        .join("/")
}

fn deserialize_api_path<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(normalize_api_path(&s))
}

/// Args of an `http::on-request` observer: the route it serves. `method` and
/// `path` are required and unknown fields are refused, so the engine answers
/// a typo with `INVALID_ARGS`; `path` is normalized (`/tasks/{tid}` becomes
/// `/tasks/:tid`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpRouteArgs {
    /// HTTP method of the route (`GET`, `POST`, …), case insensitive.
    pub method: String,
    /// Route template, e.g. `/users/:id` or `/users/{id}`. `:name` segments
    /// arrive in `path_params`.
    #[serde(deserialize_with = "deserialize_api_path")]
    pub path: String,
    /// Also expose this route on the restricted webhook listener. Explicit
    /// opt-in only; the default keeps the route on the normal listener.
    #[serde(default)]
    pub public_webhook: bool,
    /// Functions run before the target, in list order, after any global
    /// middleware.
    #[serde(default)]
    pub middleware_function_ids: Vec<String>,
}

/// Out-of-band control messages a function can send on the response channel
/// to influence the in-flight HTTP response (status/headers) before the body
/// is finalized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    SetStatus { status_code: u16 },
    SetHeaders { headers: Value },
}

impl ControlMessage {
    /// Parse a control message from raw text, rejecting messages that fail
    /// to deserialize or carry an out-of-range status code (must be in
    /// 100..=599).
    pub fn parse(text: &str) -> Option<Self> {
        let msg: ControlMessage = serde_json::from_str(text).ok()?;
        match &msg {
            ControlMessage::SetStatus { status_code } => {
                if (100..=599).contains(status_code) {
                    Some(msg)
                } else {
                    None
                }
            }
            ControlMessage::SetHeaders { .. } => Some(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // HttpRequest
    // =========================================================================

    #[test]
    fn http_request_schema_lists_the_request_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(HttpRequest)).unwrap();
        let mut fields: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        fields.sort();
        assert_eq!(
            fields,
            [
                "body",
                "headers",
                "method",
                "path",
                "path_params",
                "query_params",
                "request_body",
                "response"
            ]
        );
    }

    #[test]
    fn inbound_request_schema_lists_what_the_server_received() {
        let schema = serde_json::to_value(schemars::schema_for!(InboundRequest)).unwrap();
        let mut fields: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        fields.sort();
        assert_eq!(
            fields,
            ["body", "headers", "method", "path", "query_params"]
        );
    }

    // =========================================================================
    // HttpResponse::from_function_return
    // =========================================================================

    #[test]
    fn http_response_defaults_status_200() {
        let resp = HttpResponse::from_function_return(json!({}));
        assert_eq!(resp.status_code, 200);
    }

    #[test]
    fn http_response_headers_from_array() {
        let value = json!({
            "headers": ["Content-Type: application/json", "X-Custom: value"],
        });
        let resp = HttpResponse::from_function_return(value);
        assert_eq!(
            resp.headers,
            vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("X-Custom".to_string(), "value".to_string()),
            ]
        );
    }

    #[test]
    fn http_response_headers_from_object() {
        let value = json!({
            "headers": { "Content-Type": "text/xml" },
        });
        let resp = HttpResponse::from_function_return(value);
        assert_eq!(resp.header("content-type"), Some("text/xml"));
    }

    #[test]
    fn http_response_defaults_empty_body() {
        let resp = HttpResponse::from_function_return(json!({"status_code": 204}));
        assert_eq!(resp.body, json!({}));
    }

    // =========================================================================
    // HttpResponse::into_axum_response
    // =========================================================================

    #[test]
    fn into_axum_response_defaults_to_json_when_no_content_type() {
        let value = json!({"status_code": 201, "body": { "ok": true }});
        let resp = HttpResponse::from_function_return(value).into_axum_response();
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
        );
    }

    #[test]
    fn into_axum_response_applies_user_content_type_for_string_body() {
        let value = json!({
            "status_code": 200,
            "headers": ["Content-Type: text/xml"],
            "body": "<Response/>",
        });
        let resp = HttpResponse::from_function_return(value).into_axum_response();
        assert_eq!(
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/xml"),
        );
    }

    // =========================================================================
    // normalize_api_path
    // =========================================================================

    #[test]
    fn normalize_api_path_converts_openapi_brace_parameters() {
        assert_eq!(normalize_api_path("/tasks/{tid}"), "/tasks/:tid");
        assert_eq!(
            normalize_api_path("/users/{userId}/tasks/{tid}"),
            "/users/:userId/tasks/:tid"
        );
    }

    #[test]
    fn normalize_api_path_preserves_colon_parameters_and_static_routes() {
        assert_eq!(normalize_api_path("/tasks/:tid"), "/tasks/:tid");
        assert_eq!(normalize_api_path("/tasks/toggle"), "/tasks/toggle");
        assert_eq!(normalize_api_path("/"), "/");
        assert_eq!(normalize_api_path(""), "");
    }

    #[test]
    fn normalize_api_path_preserves_malformed_braces() {
        assert_eq!(normalize_api_path("/tasks/{tid{foo"), "/tasks/{tid{foo");
        assert_eq!(normalize_api_path("/tasks/{tid"), "/tasks/{tid");
        assert_eq!(normalize_api_path("/tasks/tid}"), "/tasks/tid}");
        assert_eq!(normalize_api_path("/tasks/{}"), "/tasks/{}");
    }

    #[test]
    fn http_route_args_normalize_a_brace_path() {
        let args: HttpRouteArgs =
            serde_json::from_value(json!({"method": "GET", "path": "/tasks/{tid}"})).unwrap();
        assert_eq!(args.path, "/tasks/:tid");
    }

    #[test]
    fn http_route_args_preserve_a_path_without_leading_slash() {
        let args: HttpRouteArgs =
            serde_json::from_value(json!({"method": "GET", "path": "tasks/{tid}"})).unwrap();
        assert_eq!(args.path, "tasks/:tid");
    }

    // =========================================================================
    // HttpRouteArgs
    // =========================================================================

    #[test]
    fn http_route_args_require_method_and_path() {
        for args in [json!({}), json!({"path": "/a"}), json!({"method": "GET"})] {
            assert!(serde_json::from_value::<HttpRouteArgs>(args).is_err());
        }
        let args: HttpRouteArgs =
            serde_json::from_value(json!({"method": "GET", "path": "/a"})).unwrap();
        assert_eq!(args.method, "GET");
        assert_eq!(args.path, "/a");
        assert!(!args.public_webhook);
        assert!(args.middleware_function_ids.is_empty());
    }

    #[test]
    fn http_route_args_refuse_unknown_fields() {
        for args in [
            json!({"method": "GET", "path": "/a", "typo": true}),
            json!({"http_method": "GET", "api_path": "/a"}),
            json!({"method": "GET", "path": "/a", "condition_function_id": "app::check"}),
        ] {
            assert!(serde_json::from_value::<HttpRouteArgs>(args).is_err());
        }
    }

    #[test]
    fn http_route_args_parse_all_fields_and_normalize_the_path() {
        let args: HttpRouteArgs = serde_json::from_value(json!({
            "method": "post",
            "path": "/users/{userId}/tasks/{tid}",
            "public_webhook": true,
            "middleware_function_ids": ["mw-1"],
        }))
        .unwrap();
        assert_eq!(args.method, "post");
        assert_eq!(args.path, "/users/:userId/tasks/:tid");
        assert!(args.public_webhook);
        assert_eq!(args.middleware_function_ids, vec!["mw-1"]);
    }

    #[test]
    fn http_route_args_schema_is_strict_for_the_engine() {
        let schema = serde_json::to_value(schemars::schema_for!(HttpRouteArgs)).unwrap();
        let mut required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        required.sort_unstable();
        assert_eq!(required, vec!["method", "path"]);
        assert_eq!(schema["additionalProperties"], false);
    }

    // =========================================================================
    // ControlMessage
    // =========================================================================

    #[test]
    fn control_message_parses_set_status() {
        let msg = ControlMessage::parse(r#"{"type":"set_status","status_code":201}"#).unwrap();
        match msg {
            ControlMessage::SetStatus { status_code } => assert_eq!(status_code, 201),
            _ => panic!("expected SetStatus"),
        }
    }

    #[test]
    fn control_message_parses_set_headers() {
        let msg = ControlMessage::parse(r#"{"type":"set_headers","headers":{"X-A":"1"}}"#).unwrap();
        match msg {
            ControlMessage::SetHeaders { headers } => {
                assert_eq!(headers, json!({"X-A": "1"}));
            }
            _ => panic!("expected SetHeaders"),
        }
    }

    #[test]
    fn control_message_rejects_status_below_100() {
        assert!(ControlMessage::parse(r#"{"type":"set_status","status_code":50}"#).is_none());
    }

    #[test]
    fn control_message_rejects_status_above_599() {
        assert!(ControlMessage::parse(r#"{"type":"set_status","status_code":600}"#).is_none());
    }

    #[test]
    fn control_message_rejects_unknown_type() {
        assert!(ControlMessage::parse(r#"{"type":"unknown"}"#).is_none());
    }
}
