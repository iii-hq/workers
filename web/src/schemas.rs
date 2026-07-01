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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FilterType {
    Pruning,
    Bm25,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdType {
    Fixed,
    Dynamic,
}

/// Content-filter request block. `query`, `threshold`, `threshold_type`, and
/// `min_word_threshold` are optional; their default values are applied when
/// the filter runs (see the content pipeline), not at deserialization.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ContentFilter {
    #[serde(rename = "type")]
    pub kind: FilterType,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub threshold_type: Option<ThresholdType>,
    #[serde(default)]
    pub min_word_threshold: Option<u32>,
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
    /// Content filter: prunes boilerplate. When set, `body` holds the filtered
    /// output in the requested `format` (page-reading mode only).
    #[serde(default)]
    pub content_filter: Option<ContentFilter>,
    /// CSS selectors (tl subset: tag/.class/#id). Restrict rendered content to
    /// these regions. Unmatched selectors are ignored.
    #[serde(default)]
    pub target_elements: Option<Vec<String>>,
    /// Tag names / selectors to drop before rendering (e.g. "nav", "footer").
    #[serde(default)]
    pub excluded_tags: Option<Vec<String>>,
    /// Add `links: { internal, external }` to the envelope.
    #[serde(default)]
    pub include_links: Option<bool>,
    /// Add `media: { images, videos, audios }` to the envelope.
    #[serde(default)]
    pub include_media: Option<bool>,
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
        // Enrichment (filtering, scoping, link/media extraction) is page-reading
        // mode only; without `format` it would be silently ignored, so reject it.
        let wants_enrichment = self.content_filter.is_some()
            || self.target_elements.as_ref().is_some_and(|v| !v.is_empty())
            || self.excluded_tags.as_ref().is_some_and(|v| !v.is_empty())
            || self.include_links == Some(true)
            || self.include_media == Some(true);
        if wants_enrichment && self.format.is_none() {
            return Err(
                "format: required with content_filter/target_elements/excluded_tags/include_links/include_media"
                    .to_string(),
            );
        }
        if let Some(cf) = &self.content_filter {
            if let Some(t) = cf.threshold {
                match cf.kind {
                    // Pruning scores are normalized to [0,1] (a product of factors each
                    // <= 1): 0 keeps everything, 1 is strictest; outside is degenerate.
                    FilterType::Pruning => {
                        if !(0.0..=1.0).contains(&t) {
                            return Err(
                                "content_filter.threshold: pruning threshold must be in [0,1]"
                                    .to_string(),
                            );
                        }
                    }
                    FilterType::Bm25 => {
                        if t < 0.0 {
                            return Err(
                                "content_filter.threshold: bm25 threshold must be >= 0".to_string()
                            );
                        }
                    }
                }
            }
        }
        for (field, list) in [
            ("target_elements", &self.target_elements),
            ("excluded_tags", &self.excluded_tags),
        ] {
            if let Some(v) = list {
                if v.iter().any(|s| s.trim().is_empty()) {
                    return Err(format!("{field}: selectors must be non-empty strings"));
                }
            }
        }
        Ok(())
    }
}

pub fn request_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(FetchPayload))
        .expect("FetchPayload schema serializes")
}

/// Typed schema for the `web::fetch` response envelope. The handler returns a
/// raw `Value` (its shape is request-dependent), so this type exists only to
/// publish a concrete response schema: the SDK would otherwise emit the
/// permissive `AnyValue` schema, which the registry rejects. Every field is
/// optional because no single response carries all of them — success carries
/// `ok`/`status`/`headers`/`body`/…, the page-mode viewable-image variant
/// carries `content`+`details`, and failures carry `ok=false`/`error`/`message`.
#[derive(JsonSchema)]
pub struct FetchResponse {
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub message: Option<String>,
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub body: Option<String>,
    pub response_format: Option<String>,
    pub bytes_truncated: Option<bool>,
    pub content_type: Option<String>,
    pub transformed: Option<String>,
    pub json: Option<serde_json::Value>,
    pub parse_error: Option<String>,
    pub redirect_chain: Option<Vec<String>>,
    pub content: Option<serde_json::Value>,
    pub details: Option<serde_json::Value>,
    pub links: Option<serde_json::Value>,
    pub media: Option<serde_json::Value>,
}

pub fn response_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(FetchResponse))
        .expect("FetchResponse schema serializes")
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
        let p =
            payload(serde_json::json!({ "url": "https://x.test/", "method": "FETCH" })).unwrap();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_empty_url_and_nonpositive_caps() {
        assert!(payload(serde_json::json!({ "url": "" }))
            .unwrap()
            .validate()
            .is_err());
        assert!(
            payload(serde_json::json!({ "url": "https://x/", "timeout_ms": 0 }))
                .unwrap()
                .validate()
                .is_err()
        );
        assert!(
            payload(serde_json::json!({ "url": "https://x/", "max_bytes": 0 }))
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn rejects_unknown_enum_values() {
        assert!(payload(serde_json::json!({ "url": "https://x/", "format": "pdf" })).is_err());
        assert!(
            payload(serde_json::json!({ "url": "https://x/", "response_format": "xml" })).is_err()
        );
    }

    #[test]
    fn request_schema_is_object_with_url() {
        let s = request_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].get("url").is_some());
    }

    // The registry's publish gate rejects untyped (AnyValue/empty) schemas,
    // so the response schema must be a concrete object. Guards against the
    // handler's raw-`Value` return leaking an AnyValue schema again.
    #[test]
    fn response_schema_is_typed_object() {
        let s = response_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].get("ok").is_some());
        assert!(s["properties"].get("error").is_some());
    }

    #[test]
    fn parses_content_filter_pruning() {
        let p = payload(serde_json::json!({
            "url": "https://x.test/",
            "content_filter": { "type": "pruning", "threshold": 0.5 }
        }))
        .unwrap();
        let cf = p.content_filter.unwrap();
        assert_eq!(cf.kind, FilterType::Pruning);
        assert_eq!(cf.threshold, Some(0.5));
    }

    #[test]
    fn parses_selection_and_extraction_flags() {
        let p = payload(serde_json::json!({
            "url": "https://x.test/",
            "target_elements": ["article", ".main"],
            "excluded_tags": ["nav", "footer"],
            "include_links": true,
            "include_media": true
        }))
        .unwrap();
        assert_eq!(p.target_elements.unwrap(), vec!["article", ".main"]);
        assert_eq!(p.excluded_tags.unwrap(), vec!["nav", "footer"]);
        assert_eq!(p.include_links, Some(true));
        assert_eq!(p.include_media, Some(true));
    }

    #[test]
    fn rejects_pruning_threshold_out_of_range() {
        let p = payload(serde_json::json!({
            "url": "https://x.test/",
            "format": "markdown",
            "content_filter": { "type": "pruning", "threshold": 1.5 }
        }))
        .unwrap();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_enrichment_without_format() {
        // Page-mode-only fields without `format` must error, not be silently dropped.
        for field in [
            serde_json::json!({ "include_links": true }),
            serde_json::json!({ "include_media": true }),
            serde_json::json!({ "excluded_tags": ["nav"] }),
            serde_json::json!({ "target_elements": ["article"] }),
            serde_json::json!({ "content_filter": { "type": "pruning" } }),
        ] {
            let mut obj = serde_json::json!({ "url": "https://x.test/" });
            obj.as_object_mut()
                .unwrap()
                .extend(field.as_object().unwrap().clone());
            assert!(payload(obj).unwrap().validate().is_err());
        }
        // Same fields WITH format validate fine.
        assert!(payload(serde_json::json!({
            "url": "https://x.test/",
            "format": "markdown",
            "include_links": true
        }))
        .unwrap()
        .validate()
        .is_ok());
    }

    #[test]
    fn bm25_threshold_above_one_is_allowed() {
        let p = payload(serde_json::json!({
            "url": "https://x.test/",
            "format": "markdown",
            "content_filter": { "type": "bm25", "threshold": 2.5, "query": "rust" }
        }))
        .unwrap();
        p.validate().unwrap();
    }

    #[test]
    fn rejects_empty_selector() {
        let p = payload(serde_json::json!({
            "url": "https://x.test/",
            "format": "markdown",
            "target_elements": ["article", "  "]
        }))
        .unwrap();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_empty_excluded_tag() {
        let p = payload(serde_json::json!({
            "url": "https://x.test/",
            "format": "markdown",
            "excluded_tags": ["nav", "  "]
        }))
        .unwrap();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_unknown_filter_type() {
        assert!(payload(serde_json::json!({
            "url": "https://x.test/",
            "content_filter": { "type": "magic" }
        }))
        .is_err());
    }
}
