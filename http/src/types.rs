//! HTTP request/response types, trigger configuration, and control messages
//! exchanged between the worker and the function it invokes.

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

/// Metadata describing the trigger that produced an `HttpRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TriggerMetadata {
    #[serde(rename = "type")]
    pub trigger_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

/// Request payload sent to the invoked function over the wire.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpRequest {
    pub query_params: HashMap<String, String>,
    pub path_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub path: String,
    pub method: String,
    pub body: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerMetadata>,

    pub request_body: StreamChannelRef,
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

fn default_http_method() -> String {
    "GET".to_string()
}

/// Configuration carried by an `http` trigger instance, as read from the
/// trigger's `config` value.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpTriggerConfig {
    pub api_path: String,
    #[serde(default = "default_http_method")]
    pub http_method: String,
    #[serde(default)]
    pub condition_function_id: Option<String>,
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
    // TriggerMetadata
    // =========================================================================

    #[test]
    fn trigger_metadata_serialize_skips_none() {
        let meta = TriggerMetadata {
            trigger_type: "cron".to_string(),
            path: None,
            method: None,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["type"], "cron");
        assert!(json.get("path").is_none());
        assert!(json.get("method").is_none());
    }

    #[test]
    fn trigger_metadata_deserialize() {
        let json = json!({"type": "http", "path": "/events", "method": "POST"});
        let meta: TriggerMetadata = serde_json::from_value(json).unwrap();
        assert_eq!(meta.trigger_type, "http");
        assert_eq!(meta.path, Some("/events".to_string()));
        assert_eq!(meta.method, Some("POST".to_string()));
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
    // HttpTriggerConfig
    // =========================================================================

    #[test]
    fn http_trigger_config_defaults_method_to_get() {
        let cfg: HttpTriggerConfig = serde_json::from_value(json!({"api_path": "/foo"})).unwrap();
        assert_eq!(cfg.api_path, "/foo");
        assert_eq!(cfg.http_method, "GET");
        assert!(cfg.condition_function_id.is_none());
        assert!(cfg.middleware_function_ids.is_empty());
    }

    #[test]
    fn http_trigger_config_parses_all_fields() {
        let cfg: HttpTriggerConfig = serde_json::from_value(json!({
            "api_path": "/foo",
            "http_method": "POST",
            "condition_function_id": "fn-1",
            "middleware_function_ids": ["mw-1", "mw-2"],
        }))
        .unwrap();
        assert_eq!(cfg.http_method, "POST");
        assert_eq!(cfg.condition_function_id, Some("fn-1".to_string()));
        assert_eq!(cfg.middleware_function_ids, vec!["mw-1", "mw-2"]);
    }

    #[test]
    fn http_trigger_config_requires_api_path() {
        let result: Result<HttpTriggerConfig, _> = serde_json::from_value(json!({}));
        assert!(result.is_err());
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
