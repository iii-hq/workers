//! The HTTP call for `web::fetch`: SSRF-checked, IP-pinned, manual redirect
//! loop with per-hop re-validation, byte-capped streaming read, and
//! page-reading transforms. `execute_fetch` lands in Task 9; this file
//! starts with the pure helpers.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::config::WebConfig;
use crate::schemas::{FetchPayload, ResponseFormat};

const DENY_ON_REDIRECT: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

/// Default to `default_timeout_ms`, never exceed `max_timeout_ms`.
pub fn resolve_timeout(p: &FetchPayload, cfg: &WebConfig) -> u64 {
    p.timeout_ms.unwrap_or(cfg.default_timeout_ms).min(cfg.max_timeout_ms)
}

/// Raw fetches default to the hard ceiling; page mode defaults to the
/// context-safe `default_response_bytes`. Never exceed `max_response_bytes`.
pub fn resolve_max_bytes(p: &FetchPayload, cfg: &WebConfig) -> u64 {
    let fallback = if p.format.is_some() {
        cfg.default_response_bytes
    } else {
        cfg.max_response_bytes
    };
    p.max_bytes.unwrap_or(fallback).min(cfg.max_response_bytes)
}

/// If `json` is set, stringify it into the body and set content-type to
/// application/json (only when the caller didn't set one). `json` wins.
pub fn apply_json_payload(
    p: &FetchPayload,
    mut headers: BTreeMap<String, String>,
) -> (Option<String>, BTreeMap<String, String>) {
    let Some(json) = &p.json else {
        return (p.body.clone(), headers);
    };
    let has_ct = headers.keys().any(|k| k.eq_ignore_ascii_case("content-type"));
    if !has_ct {
        headers.insert("content-type".to_string(), "application/json".to_string());
    }
    (Some(json.to_string()), headers)
}

/// Strip auth/cookie when the redirect leaves the origin OR downgrades
/// https→http (a same-host downgrade still leaks creds in cleartext).
pub fn strip_cross_origin_auth(
    headers: BTreeMap<String, String>,
    from: &reqwest::Url,
    to: &reqwest::Url,
) -> BTreeMap<String, String> {
    let same_host = from.host_str() == to.host_str()
        && from.port_or_known_default() == to.port_or_known_default();
    let downgrade = from.scheme() == "https" && to.scheme() == "http";
    if same_host && !downgrade {
        return headers;
    }
    headers
        .into_iter()
        .filter(|(k, _)| !DENY_ON_REDIRECT.contains(&k.to_lowercase().as_str()))
        .collect()
}

pub fn encode_body(bytes: &[u8], format: ResponseFormat) -> String {
    match format {
        ResponseFormat::Base64 => STANDARD.encode(bytes),
        // text and json both return raw utf8 in `body`; json parsing is additive.
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub fn try_parse_json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebConfig;
    use crate::schemas::{FetchPayload, ResponseFormat};
    use std::collections::BTreeMap;

    fn p(j: serde_json::Value) -> FetchPayload {
        serde_json::from_value(j).unwrap()
    }

    #[test]
    fn timeout_defaults_and_caps() {
        let cfg = WebConfig::default();
        assert_eq!(resolve_timeout(&p(serde_json::json!({"url":"x"})), &cfg), 30_000);
        assert_eq!(resolve_timeout(&p(serde_json::json!({"url":"x","timeout_ms":5000})), &cfg), 5_000);
        assert_eq!(resolve_timeout(&p(serde_json::json!({"url":"x","timeout_ms":999999})), &cfg), 120_000);
    }

    #[test]
    fn max_bytes_defaults_by_page_mode() {
        let cfg = WebConfig::default();
        assert_eq!(resolve_max_bytes(&p(serde_json::json!({"url":"x"})), &cfg), 5 * 1024 * 1024);
        assert_eq!(resolve_max_bytes(&p(serde_json::json!({"url":"x","format":"markdown"})), &cfg), 256 * 1024);
        assert_eq!(resolve_max_bytes(&p(serde_json::json!({"url":"x","max_bytes":999999999u64})), &cfg), 5 * 1024 * 1024);
    }

    #[test]
    fn json_payload_sets_body_and_content_type() {
        let pl = p(serde_json::json!({"url":"x","json":{"a":1}}));
        let (body, headers) = apply_json_payload(&pl, BTreeMap::new());
        assert_eq!(body.unwrap(), "{\"a\":1}");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn json_payload_respects_caller_content_type() {
        let pl = p(serde_json::json!({"url":"x","json":{"a":1}}));
        let mut h = BTreeMap::new();
        h.insert("content-type".to_string(), "application/vnd.x".to_string());
        let (_, headers) = apply_json_payload(&pl, h);
        assert_eq!(headers.get("content-type").unwrap(), "application/vnd.x");
    }

    #[test]
    fn strip_creds_on_host_change_and_downgrade() {
        let mut h = BTreeMap::new();
        h.insert("authorization".to_string(), "Bearer t".to_string());
        h.insert("cookie".to_string(), "s=1".to_string());
        let same = strip_cross_origin_auth(h.clone(), &"https://a.test/".parse().unwrap(), &"https://a.test/2".parse().unwrap());
        assert!(same.contains_key("authorization"));
        let cross = strip_cross_origin_auth(h.clone(), &"https://a.test/".parse().unwrap(), &"https://b.test/".parse().unwrap());
        assert!(!cross.contains_key("authorization"));
        let downgrade = strip_cross_origin_auth(h, &"https://a.test/".parse().unwrap(), &"http://a.test/".parse().unwrap());
        assert!(!downgrade.contains_key("cookie"));
    }

    #[test]
    fn encode_body_base64_and_text() {
        assert_eq!(encode_body(b"hi", ResponseFormat::Text), "hi");
        assert_eq!(encode_body(b"hi", ResponseFormat::Base64), "aGk=");
    }

    #[test]
    fn parse_json_ok_and_err() {
        assert!(try_parse_json("{\"a\":1}").is_ok());
        assert!(try_parse_json("not json").is_err());
    }
}
