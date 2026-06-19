//! Config for `web::fetch`. Numeric caps are HARD ceilings — a caller may
//! ask for less per call, never more. All fields are read per request, so
//! every field hot-reloads (no topology/restart partition).

use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WebConfig>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WebConfig {
    /// Per-request timeout used when the caller omits `timeout_ms`.
    pub default_timeout_ms: u64,
    /// Hard ceiling on per-request timeout.
    pub max_timeout_ms: u64,
    /// Body cap used in page-reading mode when the caller omits `max_bytes`.
    pub default_response_bytes: u64,
    /// Hard ceiling on response body bytes accepted before truncation.
    pub max_response_bytes: u64,
    /// Max HTML body size the page-reading transforms will process.
    pub max_transform_bytes: u64,
    /// Max redirect hops before giving up.
    pub max_redirects: u32,
    /// User-Agent we identify ourselves as outside page-reading mode.
    pub user_agent: String,
    /// Allow requests to loopback (`127.0.0.0/8`, `::1`). Other private
    /// ranges stay blocked regardless.
    pub allow_loopback: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            default_response_bytes: 256 * 1024,
            max_response_bytes: 5 * 1024 * 1024,
            max_transform_bytes: 1024 * 1024,
            max_redirects: 5,
            user_agent: "iii-harness/0.1 (+web::fetch)".to_string(),
            allow_loopback: true,
        }
    }
}

impl WebConfig {
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(WebConfig)).expect("WebConfig schema serializes")
    }

    /// Parse from a JSON object; missing keys fall back to defaults
    /// (`#[serde(default)]`). The configuration worker may store the value
    /// under a `web` wrapper or flat — accept both.
    pub fn from_json(v: &serde_json::Value) -> Result<WebConfig, String> {
        let inner = v.get("web").unwrap_or(v);
        serde_json::from_value(inner.clone()).map_err(|e| format!("invalid web config: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("WebConfig serializes")
    }

    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = WebConfig::default();
        assert_eq!(c.default_timeout_ms, 30_000);
        assert_eq!(c.max_timeout_ms, 120_000);
        assert_eq!(c.default_response_bytes, 256 * 1024);
        assert_eq!(c.max_response_bytes, 5 * 1024 * 1024);
        assert_eq!(c.max_transform_bytes, 1024 * 1024);
        assert_eq!(c.max_redirects, 5);
        assert_eq!(c.user_agent, "iii-harness/0.1 (+web::fetch)");
        assert!(c.allow_loopback);
    }

    #[test]
    fn json_roundtrip() {
        let c = WebConfig { max_redirects: 2, allow_loopback: false, ..WebConfig::default() };
        let back = WebConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.max_redirects, 2);
        assert!(!back.allow_loopback);
    }

    #[test]
    fn from_json_fills_missing_with_defaults() {
        let v = serde_json::json!({ "max_redirects": 9 });
        let c = WebConfig::from_json(&v).unwrap();
        assert_eq!(c.max_redirects, 9);
        assert_eq!(c.max_timeout_ms, 120_000); // default preserved
    }

    #[test]
    fn schema_lists_fields() {
        let s = WebConfig::json_schema();
        let props = &s["properties"];
        assert!(props.get("user_agent").is_some());
        assert!(props.get("allow_loopback").is_some());
    }
}
