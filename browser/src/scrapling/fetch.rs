//! `browser::fetch` — the no-browser HTTP tier (core.py's
//! `fetch_raw(tier="http")`).
//!
//! Safe mode uses `reqwest` + rustls and refuses options that would pretend to
//! provide curl-cffi wire parity or bypass its egress/TLS policy. Certified
//! Tier-1 compat builds instead link the frozen curl-impersonate engine behind
//! the `scrapling-compat` feature.
//!
//! In safe mode redirects are followed by hand (`Policy::none`) so every hop
//! goes back through the SSRF check — the initial URL being public says
//! nothing about where hop 3 points. Each hop also pins the socket to the
//! address we validated, which closes the DNS-rebinding window between the
//! check and the connect.

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::scrapling::page::PageData;
use crate::ssrf::{check_target, parse_target, SsrfPolicy};

#[cfg(feature = "scrapling-compat")]
mod curl_compat;
#[cfg(feature = "scrapling-compat")]
pub(crate) use curl_compat::CompatSession;

/// Chrome-ish default headers. The python worker's `stealthy_headers` (on by
/// default) adds a realistic header set plus a Google referer; this is the
/// header-level part of that, which is all we can honestly offer.
const DEFAULT_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/131.0.0.0 Safari/537.36";
const DEFAULT_ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8";

/// Ceilings on caller-supplied durations. `Duration::from_secs_f64` panics on
/// unrepresentable input, and an hour is already far past any sane fetch.
const MAX_TIMEOUT_SECS: f64 = 3_600.0;
const MAX_RETRY_DELAY_SECS: f64 = 60.0;
/// Total wall-clock budget for one `fetch` call. Without it, `timeout` is
/// per-request and multiplies out across redirect hops and retries — the
/// reference passes `timeout` to curl's `CURLOPT_TIMEOUT`, which is a TOTAL
/// budget, so a caller asking for 30s must not be able to wait 46 minutes.
const TOTAL_BUDGET_MULTIPLIER: u32 = 3;
/// Upper clamp for `max_redirects` on the safe tier. The schema is a bare
/// integer, so oversized values must clamp rather than panic; anything past
/// this is a redirect loop, not a fetch.
const MAX_REDIRECTS: i64 = 100;
/// Cap on a single response body. Without it one URL can OOM the worker and
/// take every live browser session with it.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Headers that must not survive a redirect to a different origin. reqwest
/// strips these itself when it follows redirects; we follow them by hand (to
/// re-check each hop against the SSRF policy), so stripping is ours to do.
/// curl has done this since CVE-2018-1000007.
fn is_sensitive_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "authorization" | "cookie" | "proxy-authorization" | "www-authenticate"
    )
}

/// Same origin = same scheme, host and effective port.
fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn redirected_request(status: u16, method: &str, send_body: bool) -> (String, bool) {
    if matches!(status, 301..=303) {
        (
            if method == "post" { "get" } else { method }.to_string(),
            false,
        )
    } else {
        (method.to_string(), send_body)
    }
}

#[derive(Clone, Debug)]
pub struct HttpOptions {
    pub mode: HttpMode,
    pub method: String,
    pub timeout: Duration,
    /// Preserve curl-cffi's signed millisecond value for compat setopt error
    /// parity. Safe mode only consumes the bounded `Duration` above.
    pub compat_timeout_ms: i64,
    pub follow_redirects: bool,
    pub compat_follow_redirects: i64,
    pub max_redirects: i64,
    pub retries: i64,
    pub retry_delay: Duration,
    pub retry_delay_negative: bool,
    pub proxy: Option<String>,
    pub proxies: Vec<(String, String)>,
    pub proxy_auth: Option<(String, String)>,
    pub auth: Option<(String, String)>,
    pub impersonate: Option<String>,
    pub http3: bool,
    pub verify: bool,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: Option<Body>,
    /// Shared cookie jar for `session-fetch`. Each hop still builds its own
    /// pinned client, so the jar is what carries cookies across requests (and
    /// across redirects) on a persistent HTTP session.
    pub jar: Option<std::sync::Arc<reqwest::cookie::Jar>>,
    #[cfg(feature = "scrapling-compat")]
    pub(crate) compat_session: Option<CompatSession>,
}

#[derive(Clone, Debug)]
pub enum Body {
    Form(Value),
    Json(Value),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMode {
    Safe,
    Compat,
}

fn str_pairs(payload: &Value, key: &str) -> Vec<(String, String)> {
    payload
        .get(key)
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    let s = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), s)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn python_scalar(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_string(),
        Value::Null => "None".to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => python_repr(value),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => {
            let quote = if value.contains('\'') && !value.contains('"') {
                '"'
            } else {
                '\''
            };
            let escaped = value
                .replace('\\', "\\\\")
                .replace(quote, &format!("\\{quote}"));
            format!("{quote}{escaped}{quote}")
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(name, value)| format!(
                    "{}: {}",
                    python_repr(&Value::String(name.clone())),
                    python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn python_json(value: &Value) -> String {
    match value {
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_json)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(name, value)| format!(
                    "{}: {}",
                    serde_json::to_string(name).expect("JSON string serialization cannot fail"),
                    python_json(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

fn params_pairs(payload: &Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some(params) = payload.get("params").and_then(Value::as_object) {
        for (name, value) in params {
            if let Value::Array(values) = value {
                pairs.extend(
                    values
                        .iter()
                        .map(|value| (name.clone(), python_scalar(value))),
                );
            } else {
                let value = if matches!(value, Value::Bool(_) | Value::Object(_)) {
                    python_json(value)
                } else {
                    python_scalar(value)
                };
                pairs.push((name.clone(), value));
            }
        }
    }
    pairs
}

fn pair(payload: &Value, key: &str) -> Result<Option<(String, String)>, String> {
    let Some(value) = payload.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{key} must be [username, password]"))?;
    if values.len() < 2 {
        return Err(format!(
            "not enough values to unpack (expected 2, got {})",
            values.len()
        ));
    }
    if values.len() > 2 {
        return Err("too many values to unpack (expected 2)".to_string());
    }
    let string = |index: usize| {
        values[index]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{key} must be [username, password]"))
    };
    Ok(Some((string(0)?, string(1)?)))
}

impl HttpOptions {
    /// Read the request. Defaults mirror scrapling's own
    /// (`engines/static.py`): 30s timeout, 3 attempts, 1s between them,
    /// 30 redirect hops.
    pub fn from_payload(payload: &Value) -> Result<Self, String> {
        Self::from_payload_for_mode(payload, HttpMode::Safe)
    }

    pub fn from_payload_for_mode(payload: &Value, mode: HttpMode) -> Result<Self, String> {
        if mode == HttpMode::Compat && !cfg!(feature = "scrapling-compat") {
            return Err(
                "browser::fetch compat HTTP engine is not compiled into this binary; rebuild the Tier-1 target with feature `scrapling-compat` and the certified curl-impersonate artifacts"
                    .to_string(),
            );
        }
        if mode == HttpMode::Safe {
            let reject = |field: &str, enabled: bool, reason: &str| -> Result<(), String> {
                if enabled {
                    Err(format!(
                        "safe mode refuses `{field}`: {reason}; use a certified compat build or remove the option"
                    ))
                } else {
                    Ok(())
                }
            };
            if let Some(value) = payload.get("impersonate").filter(|value| !value.is_null()) {
                let value = value.as_str().ok_or("impersonate must be a string")?;
                reject(
                    "impersonate",
                    !matches!(value, "" | "chrome"),
                    "the safe engine implements only its bounded Chrome header profile",
                )?;
            }
            reject(
                "http3",
                payload
                    .get("http3")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "the safe reqwest engine has no certified HTTP/3 transport",
            )?;
            reject(
                "verify:false",
                payload.get("verify").and_then(Value::as_bool) == Some(false),
                "TLS certificate verification cannot be disabled",
            )?;
            reject(
                "proxy",
                payload
                    .get("proxy")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                "a caller proxy can resolve or route to addresses outside the egress policy",
            )?;
            reject(
                "proxies",
                payload
                    .get("proxies")
                    .and_then(Value::as_object)
                    .is_some_and(|value| !value.is_empty()),
                "per-scheme proxies bypass address pinning",
            )?;
            reject(
                "proxy_auth",
                payload.get("proxy_auth").is_some_and(|v| !v.is_null()),
                "proxy authentication is unavailable when caller proxies are refused",
            )?;
            reject(
                "stealthy_headers:true",
                payload.get("stealthy_headers").and_then(Value::as_bool) == Some(true),
                "the safe reqwest engine cannot reproduce BrowserForge's generated header fingerprint",
            )?;
        }
        let method = payload
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("get")
            .to_ascii_lowercase();
        if !matches!(method.as_str(), "get" | "post" | "put" | "delete") {
            return Err(format!("unsupported method: {method}"));
        }
        // `timeout` is SECONDS on this tier (it is milliseconds on the browser
        // tiers — the schemas say so explicitly, and the two disagree on
        // purpose because the underlying libraries do).
        //
        // The upper clamp is not cosmetic: `Duration::from_secs_f64` PANICS on
        // a value it cannot represent, and the schema declares a bare
        // `number`, so `{"timeout": 1e20}` would panic inside the handler.
        // The SDK spawns handlers detached, so that panic would drop the
        // invocation entirely and hang the caller with no result.
        let timeout = payload
            .get("timeout")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(30.0);
        let compat_timeout_ms = (timeout * 1_000.0) as i64;
        let timeout = if mode == HttpMode::Safe {
            if timeout > 0.0 {
                timeout.min(MAX_TIMEOUT_SECS)
            } else {
                30.0
            }
        } else {
            timeout.clamp(0.0, MAX_TIMEOUT_SECS)
        };
        let raw_retry_delay = payload
            .get("retry_delay")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(1.0);
        let retry_delay_negative = raw_retry_delay < 0.0;
        let retry_delay = if mode == HttpMode::Safe {
            raw_retry_delay.clamp(0.0, MAX_RETRY_DELAY_SECS)
        } else {
            raw_retry_delay.clamp(0.0, (u64::MAX / 2) as f64)
        };
        let params = params_pairs(payload);
        let body = match (payload.get("json"), payload.get("data")) {
            (Some(j), _) if !j.is_null() => Some(Body::Json(j.clone())),
            (_, Some(d)) if !d.is_null() => Some(Body::Form(d.clone())),
            _ => None,
        };
        let raw_max_redirects = payload
            .get("max_redirects")
            .and_then(Value::as_i64)
            .unwrap_or(30);
        if mode == HttpMode::Safe && raw_max_redirects < 0 {
            return Err(format!(
                "safe mode refuses `max_redirects:{raw_max_redirects}`: unlimited or invalid redirect counts exceed the bounded request policy; use a non-negative limit"
            ));
        }
        let explicit_impersonate = payload
            .get("impersonate")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if mode == HttpMode::Compat
            && payload
                .get("proxy")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
            && payload
                .get("proxies")
                .and_then(Value::as_object)
                .is_some_and(|value| !value.is_empty())
        {
            return Err("Cannot specify both 'proxy' and 'proxies'".to_string());
        }
        let impersonate = if mode == HttpMode::Compat {
            explicit_impersonate.or_else(|| Some("chrome".to_string()))
        } else {
            None
        };
        let compat_default_ua = (mode == HttpMode::Compat)
            .then(crate::scrapling::browserforge::default_user_agent)
            .transpose()?;
        let impersonate = impersonate.filter(|value| !value.is_empty());
        let headers = stealthy_headers(
            payload,
            mode,
            impersonate.is_some(),
            compat_default_ua.as_deref(),
        )?;
        Ok(Self {
            mode,
            method,
            timeout: Duration::from_secs_f64(timeout),
            compat_timeout_ms,
            follow_redirects: payload
                .get("follow_redirects")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            compat_follow_redirects: match payload.get("follow_redirects").and_then(Value::as_bool)
            {
                Some(true) => 1,
                Some(false) => 0,
                None => 4,
            },
            max_redirects: raw_max_redirects,
            retries: payload.get("retries").and_then(Value::as_i64).unwrap_or(3),
            retry_delay: Duration::from_secs_f64(retry_delay),
            retry_delay_negative,
            proxy: payload
                .get("proxy")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
            proxies: str_pairs(payload, "proxies"),
            proxy_auth: pair(payload, "proxy_auth")?,
            auth: pair(payload, "auth")?,
            impersonate,
            http3: payload
                .get("http3")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            verify: payload
                .get("verify")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            headers,
            cookies: str_pairs(payload, "cookies"),
            params,
            body,
            jar: None,
            #[cfg(feature = "scrapling-compat")]
            compat_session: None,
        })
    }

    fn method_allows_body(&self) -> bool {
        self.method != "get"
    }
}

/// Caller headers, with browser-ish defaults filled in underneath unless
/// `stealthy_headers: false`. Caller-supplied values always win.
fn stealthy_headers(
    payload: &Value,
    mode: HttpMode,
    impersonation_enabled: bool,
    compat_default_ua: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let mut headers = str_pairs(payload, "headers");
    let stealth = payload
        .get("stealthy_headers")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let has = |h: &[(String, String)], name: &str| {
        h.iter().any(|(key, _)| key.eq_ignore_ascii_case(name))
    };
    if !stealth {
        if mode == HttpMode::Compat && !impersonation_enabled && !has(&headers, "user-agent") {
            headers.push((
                "User-Agent".into(),
                compat_default_ua.unwrap_or(DEFAULT_UA).into(),
            ));
        }
        return Ok(headers);
    }
    if mode == HttpMode::Compat && impersonation_enabled {
        if !has(&headers, "referer") {
            headers.push(("referer".into(), "https://www.google.com/".into()));
        }
    } else if mode == HttpMode::Compat {
        let supplied = headers
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();
        if !supplied.contains("referer") {
            headers.push(("referer".into(), "https://www.google.com/".into()));
        }
        for (name, value) in crate::scrapling::browserforge::generate_http_headers()? {
            if !supplied.contains(&name.to_ascii_lowercase()) {
                headers.push((name, value));
            }
        }
    } else {
        if !has(&headers, "user-agent") {
            headers.push(("user-agent".into(), DEFAULT_UA.into()));
        }
        if !has(&headers, "accept") {
            headers.push(("accept".into(), DEFAULT_ACCEPT.into()));
        }
        if !has(&headers, "accept-language") {
            headers.push(("accept-language".into(), "en-US,en;q=0.9".into()));
        }
    }
    Ok(headers)
}

fn build_client(
    hostname: &str,
    address: SocketAddr,
    opts: &HttpOptions,
) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        // Manual redirects: every hop is re-validated (see module docs).
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .timeout(opts.timeout);
    if let Some(jar) = &opts.jar {
        b = b.cookie_provider(jar.clone());
    }
    match &opts.proxy {
        // Through a proxy we cannot pin the socket — the proxy does its own
        // resolution, so `.resolve()` would be ignored and the pin would be a
        // false comfort. The proxy ENDPOINT itself is blocklist-checked
        // separately (see `check_proxy`): it is the address this worker
        // actually dials, so validating only the target would leave an
        // internal proxy usable as a pivot into the private network.
        Some(p) => {
            b = b.proxy(reqwest::Proxy::all(p).map_err(|e| format!("invalid proxy: {e}"))?);
        }
        None => {
            b = b.resolve(hostname, address);
        }
    }
    b.build().map_err(|e| e.to_string())
}

fn charset_of(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let ct = headers.get(reqwest::header::CONTENT_TYPE)?.to_str().ok()?;
    ct.split(';')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, v)| v.trim().trim_matches('"').to_ascii_lowercase())
}

#[cfg(feature = "scrapling-compat")]
fn charset_of_raw(headers: &Map<String, Value>) -> Option<String> {
    let content_type = headers.get("content-type")?.as_str()?;
    content_type
        .split(';')
        .filter_map(|part| part.split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim().trim_matches('"').to_ascii_lowercase())
}

fn decode_body(bytes: &[u8], encoding: Option<&str>) -> String {
    let decoder = encoding
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    decoder.decode(bytes).0.into_owned()
}

fn checked_body_len(current: usize, incoming: usize) -> Option<usize> {
    current
        .checked_add(incoming)
        .filter(|total| *total <= MAX_BODY_BYTES)
}

async fn bounded_body(mut response: reqwest::Response, url: &str) -> Result<Vec<u8>, String> {
    if let Some(len) = response.content_length() {
        if len > MAX_BODY_BYTES as u64 {
            return Err(format!(
                "response body is {len} bytes, over the {MAX_BODY_BYTES}-byte cap ({url})"
            ));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("reading body of {url}: {error}"))?
    {
        if checked_body_len(body.len(), chunk.len()).is_none() {
            return Err(format!(
                "response body exceeded the {MAX_BODY_BYTES}-byte cap while streaming ({url})"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn flatten_headers(headers: &reqwest::header::HeaderMap) -> Map<String, Value> {
    // Repeated headers (Set-Cookie, Vary, Link, ...) are joined with ", ",
    // matching what the compat/curl engine produces — last-value-wins would
    // silently drop data.
    let mut out = Map::new();
    for k in headers.keys() {
        let joined = headers
            .get_all(k)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ");
        out.insert(k.as_str().to_string(), json!(joined));
    }
    out
}

/// Cookies the response set, as a flat name -> value map.
fn response_cookies(headers: &reqwest::header::HeaderMap) -> Map<String, Value> {
    let mut out = Map::new();
    for v in headers.get_all(reqwest::header::SET_COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        let pair = s.split(';').next().unwrap_or("");
        if let Some((name, value)) = pair.split_once('=') {
            out.insert(name.trim().to_string(), json!(value.trim()));
        }
    }
    out
}

/// Validate a caller-supplied proxy endpoint against the same blocklist the
/// targets go through. When a proxy is configured it — not the target — is
/// what this worker connects to, so skipping this check would let
/// `{"proxy": "http://10.0.0.5:3128"}` reach straight into the private
/// network while the target check looked at an unrelated public host.
///
/// `parse_target` only admits http/https, which also rejects `socks5h://…`
/// (whose whole point is proxy-side DNS we cannot inspect).
pub async fn check_proxy(opts: &HttpOptions, policy: &SsrfPolicy) -> Result<(), String> {
    let Some(raw) = &opts.proxy else {
        return Ok(());
    };
    let target =
        parse_target(raw).map_err(|e| format!("proxy is not a usable http(s) endpoint: {e}"))?;
    check_target(&target, policy)
        .await
        .map_err(|r| format!("proxy refused: {}", r.message))?;
    Ok(())
}

/// One attempt: walk the redirect chain, SSRF-checking every hop.
async fn attempt(
    url: &str,
    opts: &HttpOptions,
    policy: &SsrfPolicy,
    deadline: std::time::Instant,
) -> Result<PageData, String> {
    let mut current = url.to_string();
    let mut method = opts.method.clone();
    let mut send_body = opts.method_allows_body();
    let mut cookies = Map::new();
    // Cookies set by responses along the chain, replayed host-only: a cookie
    // is only sent back to the exact host that set it. That covers
    // login/consent flows that bounce through the same host without ever
    // leaking a value cross-host.
    let mut hop_cookies: std::collections::HashMap<String, Map<String, Value>> =
        std::collections::HashMap::new();
    let origin = parse_target(url)?.url.clone();
    // Schema declares a bare integer; an oversized value must clamp, not
    // panic in try_from. Negative is rejected at parse for safe mode.
    let max_redirects = opts.max_redirects.clamp(0, MAX_REDIRECTS) as u32;
    for hop in 0..=max_redirects {
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "exceeded the total budget while following redirects (hop {hop}) fetching {url}"
            ));
        }
        let target = parse_target(&current)?;
        // Cross-origin hop: drop credentials the caller scoped to the origin
        // they addressed. An open redirect on a trusted host would otherwise
        // hand an Authorization bearer to whoever it points at.
        let cross_origin = !same_origin(&origin, &target.url);
        let resolved = check_target(&target, policy).await.map_err(|r| r.message)?;
        let client = build_client(
            &target.hostname,
            SocketAddr::new(resolved.address, resolved.port),
            opts,
        )?;

        let request_method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
            .map_err(|e| e.to_string())?;
        let mut req = client.request(request_method, target.url.clone());
        for (k, v) in &opts.headers {
            if cross_origin && is_sensitive_header(k) {
                continue;
            }
            req = req.header(k, v);
        }
        let mut jar = Map::new();
        // A manual Cookie header makes reqwest skip its cookie store for the
        // request, so when a session jar exists its cookies must be merged in
        // here or they would be silently suppressed by per-request cookies.
        if let Some(session_jar) = &opts.jar {
            use reqwest::cookie::CookieStore;
            if let Some(header) = session_jar.cookies(&target.url) {
                if let Ok(s) = header.to_str() {
                    for pair in s.split("; ") {
                        if let Some((k, v)) = pair.split_once('=') {
                            jar.insert(k.to_string(), json!(v));
                        }
                    }
                }
            }
        }
        if !cross_origin {
            for (k, v) in &opts.cookies {
                jar.insert(k.clone(), json!(v));
            }
        }
        // Server-set cookies from earlier hops to this same host override the
        // caller's on a name collision — that's what a cookie jar does.
        if let Some(set) = hop_cookies.get(&target.hostname) {
            for (k, v) in set {
                jar.insert(k.clone(), v.clone());
            }
        }
        if !jar.is_empty() {
            let jar = jar
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("; ");
            req = req.header(reqwest::header::COOKIE, jar);
        }
        if hop == 0 && !opts.params.is_empty() {
            req = req.query(&opts.params);
        }
        if !cross_origin {
            if let Some((username, password)) = &opts.auth {
                req = req.basic_auth(username, Some(password));
            }
        }
        // The wrapper forwards data/json for POST, PUT, and DELETE. DELETE
        // bodies are unusual but explicitly supported by Scrapling.
        if send_body {
            match &opts.body {
                Some(Body::Json(v)) => req = req.json(v),
                Some(Body::Form(v)) => {
                    let form: Vec<(String, String)> = v
                        .as_object()
                        .map(|m| {
                            m.iter()
                                .map(|(k, val)| {
                                    let s = match val {
                                        Value::String(s) => s.clone(),
                                        other => other.to_string(),
                                    };
                                    (k.clone(), s)
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    req = req.form(&form);
                }
                None => {}
            }
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                format!("timeout after {:?} fetching {current}", opts.timeout)
            } else {
                format!("transport error fetching {current}: {e}")
            }
        })?;

        let status = resp.status();
        let headers = resp.headers().clone();
        for (k, v) in response_cookies(&headers) {
            hop_cookies
                .entry(target.hostname.clone())
                .or_default()
                .insert(k.clone(), v.clone());
            cookies.insert(k, v);
        }

        if opts.follow_redirects && status.is_redirection() {
            if let Some(loc) = headers
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
            {
                (method, send_body) = redirected_request(status.as_u16(), &method, send_body);
                current = target
                    .url
                    .join(loc)
                    .map_err(|e| format!("bad redirect target {loc}: {e}"))?
                    .to_string();
                continue;
            }
        }

        let encoding = charset_of(&headers);
        let final_url = resp.url().to_string();
        let body = bounded_body(resp, &current).await?;
        let html = decode_body(&body, encoding.as_deref());
        return Ok(PageData {
            status: Some(status.as_u16()),
            url: final_url,
            headers: flatten_headers(&headers),
            cookies,
            encoding,
            html,
            captured_xhr: vec![],
        });
    }
    Err(format!(
        "too many redirects (>{}) starting at {url}",
        max_redirects
    ))
}

/// Fetch one URL, retrying transport failures. HTTP error statuses are NOT
/// retried — a 404 is an answer, and re-asking produces the same 404 while
/// costing the caller their timeout budget.
pub async fn fetch_page(
    url: &str,
    opts: &HttpOptions,
    policy: &SsrfPolicy,
) -> Result<PageData, String> {
    if opts.mode == HttpMode::Compat {
        #[cfg(feature = "scrapling-compat")]
        {
            if let Some(session) = opts.compat_session.clone() {
                return curl_compat::fetch_page_with_session(
                    url.to_string(),
                    opts.clone(),
                    session,
                )
                .await;
            }
            return curl_compat::fetch_page(url.to_string(), opts.clone()).await;
        }
        #[cfg(not(feature = "scrapling-compat"))]
        {
            return Err(
                "browser::fetch compat HTTP engine is not compiled into this binary".to_string(),
            );
        }
    }
    check_proxy(opts, policy).await?;
    // `opts.timeout` is per REQUEST; across redirect hops and retries it
    // multiplies out (3 retries x 31 hops x 30s ≈ 46 minutes by default).
    // The reference hands `timeout` to curl's CURLOPT_TIMEOUT, which is a
    // total budget, so bound the whole call too.
    let budget = opts.timeout * TOTAL_BUDGET_MULTIPLIER;
    let started = std::time::Instant::now();
    let deadline = started + budget;
    let mut last = String::new();
    let attempts = opts.retries.max(0) as u32;
    for i in 0..attempts {
        if started.elapsed() >= budget {
            return Err(if last.is_empty() {
                format!("exceeded the total budget of {budget:?} fetching {url}")
            } else {
                last
            });
        }
        match attempt(url, opts, policy, deadline).await {
            Ok(page) => return Ok(page),
            Err(e) => {
                // A refused host is a verdict, not a flake: retrying just
                // repeats the same DNS answer and the same rejection.
                if e.contains("refusing to dial")
                    || e.contains("is in ")
                    || e.starts_with("scheme not allowed")
                    || e.starts_with("url is not a valid")
                    || e.starts_with("unsupported method")
                {
                    return Err(e);
                }
                last = e;
                if i + 1 < attempts && !opts.retry_delay.is_zero() {
                    tokio::time::sleep(opts.retry_delay).await;
                }
            }
        }
    }
    if attempts == 0 {
        Err("No active session available.".to_string())
    } else {
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn options_with_proxy_for_validation(raw: &str) -> HttpOptions {
        let mut options = HttpOptions::from_payload(&json!({})).unwrap();
        options.proxy = Some(raw.to_string());
        options
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut expected = None;
        loop {
            let read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..read]);
            if expected.is_none() {
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    expected = Some(end + 4 + length);
                }
            }
            if read == 0 || expected.is_some_and(|length| request.len() >= length) {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }

    fn safe_server() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            sender.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nhe\r\n3\r\nllo\r\n0\r\n\r\n",
                )
                .unwrap();
        });
        (format!("http://{address}/safe"), receiver)
    }

    #[test]
    fn method_defaults_to_get_and_rejects_unknown() {
        assert_eq!(HttpOptions::from_payload(&json!({})).unwrap().method, "get");
        assert_eq!(
            HttpOptions::from_payload(&json!({"method": "POST"}))
                .unwrap()
                .method,
            "post"
        );
        assert_eq!(
            HttpOptions::from_payload(&json!({"method": "patch"})).unwrap_err(),
            "unsupported method: patch"
        );
    }

    #[test]
    fn safe_mode_refuses_network_options_it_cannot_enforce() {
        let cases = [
            (json!({"http3": true}), "http3"),
            (json!({"verify": false}), "verify:false"),
            (json!({"proxy": "http://proxy.test"}), "proxy"),
            (
                json!({"proxies": {"https": "http://proxy.test"}}),
                "proxies",
            ),
            (json!({"proxy_auth": ["u", "p"]}), "proxy_auth"),
            (json!({"stealthy_headers": true}), "stealthy_headers:true"),
        ];
        for (payload, option) in cases {
            let error = HttpOptions::from_payload_for_mode(&payload, HttpMode::Safe).unwrap_err();
            assert!(error.contains(option), "{option}: {error}");
        }
        assert!(HttpOptions::from_payload_for_mode(
            &json!({"impersonate": "chrome"}),
            HttpMode::Safe
        )
        .is_ok());
        assert!(HttpOptions::from_payload_for_mode(
            &json!({"impersonate": "firefox"}),
            HttpMode::Safe
        )
        .unwrap_err()
        .contains("impersonate"));
    }

    #[test]
    fn ordered_fields_and_basic_auth_are_preserved() {
        let options = HttpOptions::from_payload_for_mode(
            &json!({
                "headers": {"x-second": "2", "x-first": "1"},
                "params": {"z": "last", "a": [1, 2]},
                "cookies": {"second": "2", "first": "1"},
                "auth": ["user", "pass"]
            }),
            HttpMode::Safe,
        )
        .unwrap();
        assert_eq!(options.headers[0], ("x-second".into(), "2".into()));
        assert_eq!(options.headers[1], ("x-first".into(), "1".into()));
        assert_eq!(options.params[0], ("z".into(), "last".into()));
        assert_eq!(options.params[1], ("a".into(), "1".into()));
        assert_eq!(options.params[2], ("a".into(), "2".into()));
        assert_eq!(options.cookies[0], ("second".into(), "2".into()));
        assert_eq!(options.auth, Some(("user".into(), "pass".into())));
    }

    #[test]
    fn params_match_curl_cffi_json_and_doseq_coercion() {
        let options = HttpOptions::from_payload(&json!({
            "params": {
                "bool": true,
                "none": null,
                "obj": {"a": 1},
                "arr": [true, null, {"z": 2}]
            }
        }))
        .unwrap();
        assert_eq!(
            options.params,
            vec![
                ("bool".into(), "true".into()),
                ("none".into(), "None".into()),
                ("obj".into(), r#"{"a": 1}"#.into()),
                ("arr".into(), "True".into()),
                ("arr".into(), "None".into()),
                ("arr".into(), "{'z': 2}".into()),
            ]
        );
    }

    #[test]
    fn delete_accepts_the_same_body_inputs_as_post_and_put() {
        let options = HttpOptions::from_payload_for_mode(
            &json!({"method": "delete", "json": {"delete": true}}),
            HttpMode::Safe,
        )
        .unwrap();
        assert!(matches!(options.body, Some(Body::Json(_))));
        assert!(options.method_allows_body());
    }

    #[test]
    fn safe_refuses_unlimited_redirects_while_compat_preserves_minus_one() {
        let error =
            HttpOptions::from_payload_for_mode(&json!({"max_redirects": -1}), HttpMode::Safe)
                .unwrap_err();
        assert!(error.contains("max_redirects:-1"), "{error}");

        let options =
            HttpOptions::from_payload_for_mode(&json!({"max_redirects": -1}), HttpMode::Compat);
        if cfg!(feature = "scrapling-compat") {
            assert_eq!(options.unwrap().max_redirects, -1);
        } else {
            assert!(options.unwrap_err().contains("not compiled"));
        }
    }

    #[cfg(feature = "scrapling-compat")]
    #[test]
    fn compat_option_errors_match_the_wrapper_oracle() {
        assert_eq!(
            HttpOptions::from_payload_for_mode(
                &json!({"proxy": "http://a", "proxies": {"http": "http://b"}}),
                HttpMode::Compat,
            )
            .unwrap_err(),
            "Cannot specify both 'proxy' and 'proxies'"
        );
        assert_eq!(
            HttpOptions::from_payload_for_mode(&json!({"auth": ["u"]}), HttpMode::Compat)
                .unwrap_err(),
            "not enough values to unpack (expected 2, got 1)"
        );
    }

    #[test]
    fn timeout_is_seconds_on_this_tier() {
        let o = HttpOptions::from_payload(&json!({"timeout": 2.5})).unwrap();
        assert_eq!(o.timeout, Duration::from_millis(2500));
        // default 30s, and a nonsense value falls back rather than becoming 0
        assert_eq!(
            HttpOptions::from_payload(&json!({})).unwrap().timeout,
            Duration::from_secs(30)
        );
        assert_eq!(
            HttpOptions::from_payload(&json!({"timeout": 0}))
                .unwrap()
                .timeout,
            Duration::from_secs(30)
        );
    }

    #[test]
    fn retries_preserve_zero_attempt_quirk() {
        assert_eq!(HttpOptions::from_payload(&json!({})).unwrap().retries, 3);
        assert_eq!(
            HttpOptions::from_payload(&json!({"retries": 0}))
                .unwrap()
                .retries,
            0
        );
    }

    #[cfg(feature = "scrapling-compat")]
    #[test]
    fn compat_preserves_signed_curl_values_and_retry_quirks() {
        let timeout = HttpOptions::from_payload_for_mode(
            &json!({"timeout": -1, "retries": 1, "stealthy_headers": false}),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(timeout.compat_timeout_ms, -1000);

        let redirects = HttpOptions::from_payload_for_mode(
            &json!({"max_redirects": -2, "retries": 1, "stealthy_headers": false}),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(redirects.max_redirects, -2);

        let retries = HttpOptions::from_payload_for_mode(
            &json!({"retries": -1, "stealthy_headers": false}),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(retries.retries, -1);

        let delay = HttpOptions::from_payload_for_mode(
            &json!({"retry_delay": -1, "stealthy_headers": false}),
            HttpMode::Compat,
        )
        .unwrap();
        assert!(delay.retry_delay_negative);
    }

    #[cfg(feature = "scrapling-compat")]
    #[test]
    fn compat_does_not_silently_replace_explicit_empty_impersonation() {
        let generated =
            HttpOptions::from_payload_for_mode(&json!({"impersonate": ""}), HttpMode::Compat)
                .unwrap();
        assert_eq!(generated.impersonate, None);
        assert!(generated
            .headers
            .iter()
            .any(|(name, value)| name == "referer" && value == "https://www.google.com/"));
        assert!(generated
            .headers
            .iter()
            .any(|(name, value)| name == "User-Agent" && value.contains("Mozilla/5.0")));

        let without_stealth = HttpOptions::from_payload_for_mode(
            &json!({"impersonate": "", "stealthy_headers": false}),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(without_stealth.impersonate, None);
        assert_eq!(
            without_stealth.headers,
            vec![(
                "User-Agent".into(),
                crate::scrapling::browserforge::default_user_agent().unwrap()
            )]
        );
    }

    #[test]
    fn stealthy_headers_fill_defaults_but_never_override_caller() {
        let o = HttpOptions::from_payload(&json!({"headers": {"User-Agent": "mine"}})).unwrap();
        assert_eq!(o.headers[0], ("User-Agent".into(), "mine".into()));
        assert!(
            !o.headers.iter().any(|(name, _)| name == "user-agent"),
            "must not add a second, case-variant UA header"
        );
        assert!(o.headers.iter().any(|(name, _)| name == "accept"));

        let off = HttpOptions::from_payload(&json!({"stealthy_headers": false})).unwrap();
        assert!(off.headers.is_empty());
    }

    #[test]
    fn json_body_wins_over_data_and_only_on_post_put() {
        let o = HttpOptions::from_payload(&json!({"json": {"a": 1}, "data": {"b": 2}})).unwrap();
        assert!(matches!(o.body, Some(Body::Json(_))));
    }

    #[test]
    fn charset_parsed_from_content_type() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::CONTENT_TYPE,
            "text/html; charset=ISO-8859-1".parse().unwrap(),
        );
        assert_eq!(charset_of(&h).as_deref(), Some("iso-8859-1"));
        assert_eq!(charset_of(&reqwest::header::HeaderMap::new()), None);
    }

    #[test]
    fn streamed_body_limit_is_checked_even_without_content_length() {
        assert_eq!(
            checked_body_len(MAX_BODY_BYTES - 1, 1),
            Some(MAX_BODY_BYTES)
        );
        assert_eq!(checked_body_len(MAX_BODY_BYTES, 1), None);
        assert_eq!(checked_body_len(usize::MAX, 1), None);
    }

    #[test]
    fn set_cookie_headers_flatten_to_name_value() {
        let mut h = reqwest::header::HeaderMap::new();
        h.append(
            reqwest::header::SET_COOKIE,
            "sid=abc; Path=/; HttpOnly".parse().unwrap(),
        );
        h.append(
            reqwest::header::SET_COOKIE,
            "theme=dark; Max-Age=60".parse().unwrap(),
        );
        let c = response_cookies(&h);
        assert_eq!(c.get("sid").unwrap(), &json!("abc"));
        assert_eq!(c.get("theme").unwrap(), &json!("dark"));
    }

    #[test]
    fn absurd_durations_are_clamped_not_panicked_on() {
        // `Duration::from_secs_f64` panics on an unrepresentable value, and
        // the SDK spawns handlers detached — so a panic here would drop the
        // invocation and hang the caller with no result at all.
        for v in [1e20, f64::MAX, 1e308] {
            let o = HttpOptions::from_payload(&json!({"timeout": v, "retry_delay": v})).unwrap();
            assert!(o.timeout <= Duration::from_secs_f64(MAX_TIMEOUT_SECS));
            assert!(o.retry_delay <= Duration::from_secs_f64(MAX_RETRY_DELAY_SECS));
        }
        // NaN and infinity fall back to the default rather than clamping.
        let o = HttpOptions::from_payload(&json!({"timeout": f64::NAN})).unwrap();
        assert_eq!(o.timeout, Duration::from_secs(30));
    }

    #[test]
    fn sensitive_headers_are_recognised_case_insensitively() {
        for h in [
            "Authorization",
            "authorization",
            "COOKIE",
            "Proxy-Authorization",
        ] {
            assert!(is_sensitive_header(h), "{h} must be treated as sensitive");
        }
        for h in ["accept", "user-agent", "x-custom"] {
            assert!(!is_sensitive_header(h));
        }
    }

    #[test]
    fn same_origin_compares_scheme_host_and_effective_port() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(same_origin(&u("https://a.test/x"), &u("https://a.test/y")));
        // default port is the same origin as the explicit one
        assert!(same_origin(
            &u("https://a.test/"),
            &u("https://a.test:443/")
        ));
        assert!(!same_origin(&u("https://a.test/"), &u("http://a.test/")));
        assert!(!same_origin(&u("https://a.test/"), &u("https://b.test/")));
        assert!(!same_origin(
            &u("https://a.test/"),
            &u("https://a.test:8443/")
        ));
    }

    #[test]
    fn redirect_method_and_body_rules_match_curl() {
        for status in 301..=303 {
            assert_eq!(
                redirected_request(status, "post", true),
                ("get".into(), false)
            );
            for method in ["put", "delete"] {
                assert_eq!(
                    redirected_request(status, method, true),
                    (method.into(), false)
                );
            }
        }
        for status in [307, 308] {
            for method in ["post", "put", "delete"] {
                assert_eq!(
                    redirected_request(status, method, true),
                    (method.into(), true)
                );
            }
        }
    }

    #[tokio::test]
    async fn cross_origin_redirect_drops_credentials_body_and_initial_query() {
        let destination = TcpListener::bind("127.0.0.1:0").unwrap();
        let destination_address = destination.local_addr().unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_address = origin.local_addr().unwrap();
        let (sender, requests) = mpsc::channel();

        let first_sender = sender.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = origin.accept().unwrap();
            first_sender.send(read_request(&mut stream)).unwrap();
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://{destination_address}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        std::thread::spawn(move || {
            let (mut stream, _) = destination.accept().unwrap();
            sender.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });

        let options = HttpOptions::from_payload(&json!({
            "method": "post",
            "data": {"a": "b"},
            "params": {"q": 1},
            "auth": ["u", "p"],
            "cookies": {"sid": "x"},
            "headers": {"X-Test": "kept"},
            "follow_redirects": true,
            "retries": 1,
            "stealthy_headers": false
        }))
        .unwrap();
        let page = fetch_page(
            &format!("http://{origin_address}/start"),
            &options,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .unwrap();

        let first = requests.recv().unwrap();
        assert!(first.starts_with("POST /start?q=1 HTTP/1.1\r\n"), "{first}");
        assert!(first
            .to_ascii_lowercase()
            .contains("authorization: basic dtpw"));
        assert!(first.to_ascii_lowercase().contains("cookie: sid=x"));
        assert!(first.ends_with("a=b"), "{first}");

        let second = requests.recv().unwrap();
        let lower = second.to_ascii_lowercase();
        assert!(second.starts_with("GET /end HTTP/1.1\r\n"), "{second}");
        assert!(!lower.contains("authorization:"), "{second}");
        assert!(!lower.contains("cookie:"), "{second}");
        assert!(lower.contains("x-test: kept"), "{second}");
        assert!(!second.contains("?q=1"), "{second}");
        assert_eq!(page.html, "ok");
    }

    #[tokio::test]
    async fn redirect_to_metadata_is_refused_before_the_second_request() {
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_address = origin.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = origin.accept().unwrap();
            let _ = read_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let options = HttpOptions::from_payload(&json!({
            "follow_redirects": true,
            "retries": 3,
            "retry_delay": 0,
            "stealthy_headers": false
        }))
        .unwrap();
        let error = fetch_page(
            &format!("http://{origin_address}/redirect"),
            &options,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("link-local"), "{error}");
    }

    #[tokio::test]
    async fn a_private_proxy_is_refused_before_any_request() {
        // The proxy is the address this worker actually dials, so validating
        // only the (public) target would let an internal proxy be used as a
        // pivot into the private network.
        let opts = options_with_proxy_for_validation("http://169.254.169.254:3128");
        let err = check_proxy(
            &opts,
            &SsrfPolicy {
                allow_loopback: false,
            },
        )
        .await
        .unwrap_err();
        assert!(err.starts_with("proxy refused:"), "got: {err}");
        assert!(err.contains("link-local"), "got: {err}");
    }

    #[tokio::test]
    async fn a_socks_proxy_is_refused_because_its_dns_is_uninspectable() {
        let opts = options_with_proxy_for_validation("socks5h://127.0.0.1:1080");
        let err = check_proxy(
            &opts,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("not a usable http(s) endpoint"), "got: {err}");
    }

    #[tokio::test]
    async fn invalid_proxy_errors_do_not_echo_credentials() {
        let raw = "http://user:pass@[";
        let opts = options_with_proxy_for_validation(raw);
        let check_err = check_proxy(
            &opts,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .unwrap_err();
        assert!(
            !check_err.contains("user:pass"),
            "credentials leaked: {check_err}"
        );
        assert!(
            check_err.contains("not a usable http(s) endpoint"),
            "got: {check_err}"
        );

        let build_err =
            match build_client("example.com", SocketAddr::from(([1, 1, 1, 1], 443)), &opts) {
                Ok(_) => panic!("invalid proxy unexpectedly built a client"),
                Err(error) => error,
            };
        assert!(
            !build_err.contains("user:pass"),
            "credentials leaked: {build_err}"
        );
        assert!(build_err.contains("invalid proxy"), "got: {build_err}");
    }

    #[tokio::test]
    async fn no_proxy_configured_is_not_an_error() {
        let opts = HttpOptions::from_payload(&json!({})).unwrap();
        assert!(check_proxy(
            &opts,
            &SsrfPolicy {
                allow_loopback: false
            }
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn a_public_proxy_passes_the_check() {
        let opts = options_with_proxy_for_validation("http://1.1.1.1:8080");
        assert!(check_proxy(
            &opts,
            &SsrfPolicy {
                allow_loopback: false
            }
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn ssrf_rejection_is_not_retried_and_names_the_range() {
        let opts = HttpOptions::from_payload(&json!({"retries": 3, "retry_delay": 0})).unwrap();
        let policy = SsrfPolicy {
            allow_loopback: false,
        };
        let err = fetch_page("http://169.254.169.254/latest/meta-data/", &opts, &policy)
            .await
            .unwrap_err();
        assert!(err.contains("link-local"), "got: {err}");
    }

    #[tokio::test]
    async fn safe_engine_streams_unknown_length_and_sends_delete_body() {
        let (url, request) = safe_server();
        let options = HttpOptions::from_payload(&json!({
            "method": "delete",
            "json": {"delete": true},
            "stealthy_headers": false,
            "retries": 1
        }))
        .unwrap();
        let page = fetch_page(
            &url,
            &options,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .unwrap();
        let raw = request.recv().unwrap();
        assert!(raw.starts_with("DELETE /safe HTTP/1.1\r\n"), "{raw}");
        assert!(raw.ends_with("{\"delete\":true}"), "{raw}");
        assert_eq!(page.html, "hello");
    }

    #[tokio::test]
    async fn fetches_a_real_page_over_the_wire() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body = b"<html><body><h1>Example Domain</h1></body></html>";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        let url = format!("http://{address}/");
        let opts = HttpOptions::from_payload(&json!({"stealthy_headers": false})).unwrap();
        let page = fetch_page(
            &url,
            &opts,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .expect("hermetic origin should be reachable");

        assert_eq!(page.status, Some(200));
        assert_eq!(page.url, url);
        assert_eq!(
            page.html,
            "<html><body><h1>Example Domain</h1></body></html>"
        );
        assert_eq!(page.headers["content-type"], "text/html; charset=utf-8");

        // And the shared envelope on top of it: inline extraction + render.
        let out = crate::scrapling::page::serialize_page(
            &page,
            &json!({"selectors": [{"name": "title", "css": "h1"}], "format": "text"}),
            false,
        )
        .unwrap();
        assert_eq!(out["extracted"]["title"], json!("Example Domain"));
        assert_eq!(out["status"], json!(200));
        assert_eq!(out["content"], "Example Domain");
    }

    #[tokio::test]
    async fn non_http_scheme_refused() {
        let opts = HttpOptions::from_payload(&json!({})).unwrap();
        let err = fetch_page(
            "file:///etc/passwd",
            &opts,
            &SsrfPolicy {
                allow_loopback: true,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("scheme not allowed"), "got: {err}");
    }
}
