//! The HTTP call for `web::fetch`: SSRF-checked, IP-pinned, manual redirect
//! loop with per-hop re-validation, byte-capped streaming read, and
//! page-reading transforms. `execute_fetch` lands in Task 9; this file
//! starts with the pure helpers.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures::StreamExt;
use serde_json::{json, Value};

use crate::config::WebConfig;
use crate::convert::{
    accept_header_for, extract_text, html_to_markdown, is_image_mime, is_viewable_image_mime,
    max_tag_depth, ACCEPT_LANGUAGE, BROWSER_USER_AGENT, MAX_NESTING_DEPTH,
};
use crate::schemas::{FetchPayload, PageFormat, ResponseFormat};
use crate::ssrf::{check_target, parse_target, SsrfPolicy};

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

/// Read a byte stream up to `cap`, marking truncation. Once the cap is
/// reached the result is terminal — a subsequent stream error (e.g. reset
/// on drop) is ignored: the cap is the contract.
pub async fn read_capped<S>(mut stream: S, cap: u64) -> (Vec<u8>, bool)
where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
{
    let cap = cap as usize;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(item) = stream.next().await {
        let chunk = match item {
            Ok(c) => c,
            Err(_) => return (buf, false), // pre-cap transport error: surface as non-truncated; caller maps it
        };
        if buf.len() + chunk.len() > cap {
            let remaining = cap.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..remaining]);
            return (buf, true); // truncation is terminal; drop the stream
        }
        buf.extend_from_slice(&chunk);
    }
    (buf, false)
}

struct RawResponse {
    status: u16,
    status_text: String,
    headers: BTreeMap<String, String>,
    bytes: Vec<u8>,
    truncated: bool,
}

enum Outcome {
    Response(RawResponse),
    Timeout,
    Transport(String),
}

fn error_envelope(code: &str, message: impl Into<String>) -> Value {
    json!({ "ok": false, "error": code, "message": message.into() })
}

/// Flatten reqwest headers into a lower-cased string map, joining repeated
/// headers (e.g. set-cookie) with ", ".
fn flatten_headers(h: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in h.iter() {
        let k = name.as_str().to_lowercase();
        let v = value.to_str().unwrap_or("").to_string();
        out.entry(k)
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&v);
            })
            .or_insert(v);
    }
    out
}

/// Drop any caller-supplied Host header so reqwest derives it from the URL.
fn without_host(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("host"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// One HTTP request, pinned to `address`. SNI/cert/Host stay on the URL
/// hostname; only the TCP target is overridden (DNS-rebinding defense).
async fn perform_request(
    target: &crate::ssrf::ParsedTarget,
    address: SocketAddr,
    method: &str,
    headers: &BTreeMap<String, String>,
    body: Option<&str>,
    timeout: Duration,
    max_bytes: u64,
) -> Outcome {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&target.hostname, address)
        .pool_max_idle_per_host(0)
        .timeout(timeout)
        .build()
    {
        Ok(c) => c,
        Err(e) => return Outcome::Transport(e.to_string()),
    };

    let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut req = client.request(m, target.url.clone());
    for (k, v) in without_host(headers) {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        if method != "GET" && method != "HEAD" {
            req = req.body(b.to_string());
        }
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return if e.is_timeout() {
                Outcome::Timeout
            } else {
                Outcome::Transport(e.to_string())
            };
        }
    };

    let status = resp.status().as_u16();
    let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
    let headers = flatten_headers(resp.headers());
    // Box::pin so the `Unpin` bound on read_capped holds regardless of
    // reqwest's internal stream type.
    let (bytes, truncated) = read_capped(Box::pin(resp.bytes_stream()), max_bytes).await;
    Outcome::Response(RawResponse { status, status_text, headers, bytes, truncated })
}

pub async fn execute_fetch(payload: FetchPayload, cfg: &WebConfig) -> Value {
    let method = payload.normalized_method();
    let follow_redirects = payload.follow_redirects.unwrap_or(true);
    let page_format = payload.format;
    let response_format = if page_format.is_some() {
        ResponseFormat::Text
    } else {
        payload.response_format.unwrap_or(ResponseFormat::Text)
    };
    let timeout = Duration::from_millis(resolve_timeout(&payload, cfg));
    let max_bytes = resolve_max_bytes(&payload, cfg);
    let policy = SsrfPolicy { allow_loopback: cfg.allow_loopback };

    let mut current_url = payload.url.clone();

    // Build base headers: page mode injects a browser UA + Accept + Accept-Language
    // for headers the caller didn't supply; otherwise the configured UA.
    let caller_keys: std::collections::BTreeSet<String> = payload
        .headers
        .as_ref()
        .map(|h| h.keys().map(|k| k.to_lowercase()).collect())
        .unwrap_or_default();
    let browser_ua_injected = page_format.is_some() && !caller_keys.contains("user-agent");

    let mut base: BTreeMap<String, String> = BTreeMap::new();
    base.insert(
        "user-agent".to_string(),
        if browser_ua_injected { BROWSER_USER_AGENT.to_string() } else { cfg.user_agent.clone() },
    );
    if let Some(f) = page_format {
        if !caller_keys.contains("accept") {
            base.insert("accept".to_string(), accept_header_for(f).to_string());
        }
        if !caller_keys.contains("accept-language") {
            base.insert("accept-language".to_string(), ACCEPT_LANGUAGE.to_string());
        }
    }
    if let Some(h) = &payload.headers {
        for (k, v) in h {
            base.insert(k.clone(), v.clone()); // caller wins
        }
    }
    let (effective_body, mut current_headers) = apply_json_payload(&payload, base);

    let mut redirect_chain: Vec<String> = Vec::new();

    for hop in 0..=cfg.max_redirects {
        let parsed = match parse_target(&current_url) {
            Ok(p) => p,
            Err(e) => return error_envelope("invalid_url", e),
        };
        let resolved = match check_target(&parsed, &policy).await {
            Ok(r) => r,
            Err(rej) => return error_envelope(rej.code, rej.message),
        };
        let socket = SocketAddr::new(resolved.address, resolved.port);

        let mut outcome = perform_request(
            &parsed, socket, &method, &current_headers, effective_body.as_deref(), timeout, max_bytes,
        )
        .await;

        // Cloudflare cf-mitigated:challenge: retry the SAME hop once with the
        // honest UA. Only when we injected the browser UA, GET/HEAD, 403+challenge.
        if browser_ua_injected
            && (method == "GET" || method == "HEAD")
        {
            if let Outcome::Response(r) = &outcome {
                if r.status == 403 && r.headers.get("cf-mitigated").map(|s| s.as_str()) == Some("challenge") {
                    let mut retry_headers = current_headers.clone();
                    retry_headers.insert("user-agent".to_string(), cfg.user_agent.clone());
                    outcome = perform_request(
                        &parsed, socket, &method, &retry_headers, effective_body.as_deref(), timeout, max_bytes,
                    )
                    .await;
                }
            }
        }

        let resp = match outcome {
            Outcome::Timeout => {
                return error_envelope(
                    "timeout",
                    format!("request timed out after {}ms (raise timeout_ms or shrink response with smaller max_bytes)", timeout.as_millis()),
                );
            }
            Outcome::Transport(m) => return error_envelope("transport_error", m),
            Outcome::Response(r) => r,
        };

        // Redirect handling.
        if follow_redirects && (300..400).contains(&resp.status) {
            if let Some(location) = resp.headers.get("location") {
                let next = match parsed.url.join(location) {
                    Ok(u) => u,
                    Err(_) => {
                        return error_envelope(
                            "transport_error",
                            format!("redirect target is not a valid URL: {location}"),
                        );
                    }
                };
                if hop == cfg.max_redirects {
                    return error_envelope(
                        "too_many_redirects",
                        format!("exceeded max_redirects ({})", cfg.max_redirects),
                    );
                }
                redirect_chain.push(current_url.clone());
                current_headers = strip_cross_origin_auth(current_headers, &parsed.url, &next);
                current_url = next.to_string();
                continue;
            }
            // 3xx without Location: fall through and return the response.
        }

        return shape_response(resp, page_format, response_format, &redirect_chain, cfg);
    }

    error_envelope(
        "too_many_redirects",
        format!("exceeded max_redirects ({})", cfg.max_redirects),
    )
}

fn shape_response(
    resp: RawResponse,
    page_format: Option<PageFormat>,
    response_format: ResponseFormat,
    redirect_chain: &[String],
    cfg: &WebConfig,
) -> Value {
    if let Some(pf) = page_format {
        let content_type = resp.headers.get("content-type").cloned().unwrap_or_default();
        let mime = content_type.split(';').next().unwrap_or("").trim().to_lowercase();

        if is_image_mime(&mime) {
            let viewable = is_viewable_image_mime(&mime)
                && (200..300).contains(&resp.status)
                && !resp.truncated
                && !resp.bytes.is_empty();
            if viewable {
                let mut details = json!({
                    "ok": true,
                    "status": resp.status,
                    "status_text": resp.status_text,
                    "content_type": mime,
                    "bytes": resp.bytes.len(),
                });
                if !redirect_chain.is_empty() {
                    details["redirect_chain"] = json!(redirect_chain);
                }
                return json!({
                    "content": [
                        { "type": "image", "mime": mime, "data": STANDARD.encode(&resp.bytes) },
                        { "type": "text", "text": format!("Image fetched ({mime}, {} bytes)", resp.bytes.len()) },
                    ],
                    "details": details,
                });
            }
            // Non-viewable image → base64 envelope.
            let mut out = json!({
                "ok": true,
                "status": resp.status,
                "status_text": resp.status_text,
                "headers": resp.headers,
                "body": STANDARD.encode(&resp.bytes),
                "response_format": "base64",
                "bytes_truncated": resp.truncated,
                "content_type": mime,
            });
            if !redirect_chain.is_empty() {
                out["redirect_chain"] = json!(redirect_chain);
            }
            return out;
        }

        let raw = String::from_utf8_lossy(&resp.bytes).into_owned();
        let is_html = mime == "text/html" || mime == "application/xhtml+xml";
        let mut body = raw.clone();
        let mut transformed: Option<&str> = None;
        let within_cap = (resp.bytes.len() as u64) <= cfg.max_transform_bytes;
        let depth_ok = within_cap && max_tag_depth(&raw) <= MAX_NESTING_DEPTH;

        if is_html && within_cap && depth_ok && matches!(pf, PageFormat::Markdown | PageFormat::Text) {
            let converted = match pf {
                PageFormat::Markdown => html_to_markdown(&raw),
                PageFormat::Text => Ok(extract_text(&raw)),
                PageFormat::Html => unreachable!(),
            };
            if let Ok(text) = converted {
                body = text;
                transformed = Some(match pf {
                    PageFormat::Markdown => "markdown",
                    PageFormat::Text => "text",
                    PageFormat::Html => "html",
                });
                if resp.truncated {
                    body.push_str("\n\n[Content truncated at max_bytes — the page continues beyond this point]");
                }
            }
            // On transform failure: keep raw body, leave transformed unset.
        }

        let mut out = json!({
            "ok": true,
            "status": resp.status,
            "status_text": resp.status_text,
            "headers": resp.headers,
            "body": body,
            "response_format": "text",
            "bytes_truncated": resp.truncated,
            "content_type": mime,
        });
        if let Some(t) = transformed {
            out["transformed"] = json!(t);
        }
        if !redirect_chain.is_empty() {
            out["redirect_chain"] = json!(redirect_chain);
        }
        return out;
    }

    // Raw (non-page) mode.
    let body = encode_body(&resp.bytes, response_format);
    let mut out = json!({
        "ok": true,
        "status": resp.status,
        "status_text": resp.status_text,
        "headers": resp.headers,
        "body": body,
        "response_format": match response_format {
            ResponseFormat::Text => "text",
            ResponseFormat::Base64 => "base64",
            ResponseFormat::Json => "json",
        },
        "bytes_truncated": resp.truncated,
    });
    if !redirect_chain.is_empty() {
        out["redirect_chain"] = json!(redirect_chain);
    }
    if matches!(response_format, ResponseFormat::Json) && !resp.truncated {
        match try_parse_json(&body) {
            Ok(v) => out["json"] = v,
            Err(e) => out["parse_error"] = json!(e),
        }
    }
    out
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

    #[tokio::test]
    async fn read_capped_truncates_and_marks() {
        let chunks: Vec<reqwest::Result<bytes::Bytes>> = vec![
            Ok(bytes::Bytes::from_static(b"aaaa")),
            Ok(bytes::Bytes::from_static(b"bbbb")),
        ];
        let (bytes, truncated) = read_capped(futures::stream::iter(chunks), 6).await;
        assert_eq!(bytes, b"aaaabb");
        assert!(truncated);
    }

    #[tokio::test]
    async fn read_capped_full_under_cap() {
        let chunks: Vec<reqwest::Result<bytes::Bytes>> =
            vec![Ok(bytes::Bytes::from_static(b"hello"))];
        let (bytes, truncated) = read_capped(futures::stream::iter(chunks), 100).await;
        assert_eq!(bytes, b"hello");
        assert!(!truncated);
    }

    #[tokio::test]
    async fn read_capped_truncation_wins_over_later_error() {
        // a chunk that overflows the cap, then an error that must be ignored
        let chunks: Vec<reqwest::Result<bytes::Bytes>> = vec![
            Ok(bytes::Bytes::from_static(b"aaaaaaaa")),
        ];
        let (bytes, truncated) = read_capped(futures::stream::iter(chunks), 4).await;
        assert_eq!(bytes, b"aaaa");
        assert!(truncated);
    }
}
