//! Wire schema + tool description for `web::fetch`. `FetchPayload` derives
//! `JsonSchema` (for `request_schema`) and `Deserialize`. The handler takes
//! a raw `Value` and parses this so a bad payload returns `invalid_payload`
//! rather than an SDK-level error (the SDK deserializes typed inputs before
//! the handler runs).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;

pub const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResponseFormat {
    Text,
    Base64,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PageFormat {
    Markdown,
    Text,
    Html,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FetchPayload {
    /// Absolute http(s):// URL to fetch.
    pub url: String,
    /// HTTP method. Defaults to GET. Case-insensitive ("get" works).
    #[serde(default)]
    pub method: Option<String>,
    /// Request headers. Lower-cased keys recommended.
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    /// Raw request body as a string. Prefer `json` for JSON payloads.
    #[serde(default)]
    pub body: Option<String>,
    /// Structured JSON payload. When set, the handler stringifies it and
    /// forces `content-type: application/json`; wins over `body`.
    #[serde(default)]
    pub json: Option<serde_json::Value>,
    /// Per-request timeout in ms. Capped by the worker's max_timeout_ms.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Cap on response body bytes. Defaults to the worker ceiling for raw
    /// fetches, or 256 KiB in page-reading mode (`format` set).
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// Follow 3xx redirects. Defaults to true. Each hop is re-checked
    /// against the SSRF blocklist.
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    /// How to return the body: "text" (default), "base64", or "json".
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
    /// Page-reading mode: "markdown" | "text" | "html". When set, a browser
    /// UA + format Accept header are used, HTML is transformed, and
    /// response_format is ignored (treated as "text").
    #[serde(default)]
    pub format: Option<PageFormat>,
}

impl FetchPayload {
    pub fn normalized_method(&self) -> String {
        self.method
            .as_deref()
            .map(|m| m.to_uppercase())
            .unwrap_or_else(|| "GET".to_string())
    }

    /// Post-parse validation that serde can't express (positivity, method
    /// enum, non-empty url). Returns an `invalid_payload` message on failure.
    pub fn validate(&self) -> Result<(), String> {
        if self.url.is_empty() {
            return Err("url: must be a non-empty string".to_string());
        }
        if let Some(m) = &self.method {
            if !METHODS.contains(&m.to_uppercase().as_str()) {
                return Err(format!("method: must be one of {}", METHODS.join(", ")));
            }
        }
        if matches!(self.timeout_ms, Some(0)) {
            return Err("timeout_ms: must be a positive integer".to_string());
        }
        if matches!(self.max_bytes, Some(0)) {
            return Err("max_bytes: must be a positive integer".to_string());
        }
        Ok(())
    }
}

pub fn request_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(FetchPayload)).expect("FetchPayload schema serializes")
}

pub const TOOL_DESCRIPTION: &str = concat!(
    "Fetch a URL over HTTP(S) and return the response as a structured envelope. ",
    "Use this INSTEAD of `shell::exec` with curl for any HTTP request — it ",
    "returns {ok, status, headers, body} as JSON, enforces size/timeout caps, ",
    "and blocks private / cloud-metadata / link-local addresses server-side ",
    "(SSRF guard; loopback is allowed by default for harness dev workflows). ",
    "To READ A WEB PAGE, set `format: \"markdown\"` — HTML is converted to ",
    "Markdown and images come back viewable (image responses in that mode ",
    "return the image itself plus a text line, not the {ok,...} envelope). ",
    "For JSON: pass `json: {...}` (auto-stringifies + sets content-type) and ",
    "`response_format: \"json\"` (auto-parses response into the `json` field). ",
    "Method is case-insensitive. On failure returns `{ok:false, error, message}` ",
    "where `error` is one of: invalid_payload, invalid_url, blocked_host, timeout, ",
    "too_many_redirects, transport_error. Branch on `error`, not on text."
);

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(j: serde_json::Value) -> Result<FetchPayload, serde_json::Error> {
        serde_json::from_value(j)
    }

    #[test]
    fn parses_minimal() {
        let p = payload(serde_json::json!({ "url": "https://x.test/" })).unwrap();
        assert_eq!(p.url, "https://x.test/");
        assert_eq!(p.normalized_method(), "GET");
        p.validate().unwrap();
    }

    #[test]
    fn method_case_insensitive() {
        let p = payload(serde_json::json!({ "url": "https://x.test/", "method": "post" })).unwrap();
        assert_eq!(p.normalized_method(), "POST");
        p.validate().unwrap();
    }

    #[test]
    fn rejects_unknown_method() {
        let p = payload(serde_json::json!({ "url": "https://x.test/", "method": "FETCH" })).unwrap();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_empty_url_and_nonpositive_caps() {
        assert!(payload(serde_json::json!({ "url": "" })).unwrap().validate().is_err());
        assert!(payload(serde_json::json!({ "url": "https://x/", "timeout_ms": 0 })).unwrap().validate().is_err());
        assert!(payload(serde_json::json!({ "url": "https://x/", "max_bytes": 0 })).unwrap().validate().is_err());
    }

    #[test]
    fn rejects_unknown_enum_values() {
        assert!(payload(serde_json::json!({ "url": "https://x/", "format": "pdf" })).is_err());
        assert!(payload(serde_json::json!({ "url": "https://x/", "response_format": "xml" })).is_err());
    }

    #[test]
    fn request_schema_is_object_with_url() {
        let s = request_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].get("url").is_some());
    }
}
