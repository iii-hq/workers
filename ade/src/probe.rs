//! `POST /probe` — run a trigger-catalog test request from the console
//! process instead of the browser tab.
//!
//! A browser `fetch` cannot show a redirect's status code: following the 3xx
//! lands on the (often cross-origin) target and trips CORS, while
//! `redirect: 'manual'` yields an opaque `status: 0`. Making the request
//! server-side reads the real status (302 and its `Location` included) and
//! returns it same-origin, so the panel renders `302` exactly like `200`.
//!
//! SSRF note: the request body carries only the method, path, and body. The
//! target host and port are resolved server-side from the HTTP worker's own
//! configuration entry — never taken from the caller — so `/probe` can reach
//! only that worker, not an arbitrary address the console can see.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use iii_sdk::protocol::TriggerRequest;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::AppState;

/// The catalog's own method set; anything else is rejected before a request
/// is built.
const ALLOWED_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// Request headers this interface will forward. The panel only sets a content
/// type; the rest are non-credential, non-sensitive request headers a tester
/// might reasonably add. Anything outside this set is rejected rather than
/// forwarded — a caller must never be able to push `Authorization`, `Cookie`,
/// or the like over the plaintext hop to the HTTP worker.
const ALLOWED_HEADERS: &[&str] = &["content-type", "accept", "accept-language"];

/// Matches the HTTP worker's own `default_timeout`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Cap the returned body so a large endpoint response cannot balloon the
/// panel; the catalog only needs enough to confirm the shape.
const MAX_BODY_BYTES: usize = 1_000_000;

#[derive(Deserialize)]
pub struct ProbeRequest {
    method: String,
    /// The part after the base URL, e.g. `/s/home?x=1`. Must start with a
    /// single `/`; the host and port are never taken from the caller.
    path: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
}

/// Validate the caller-supplied method and path, returning the normalized
/// method. The path is concatenated straight after the server-chosen
/// `host:port`, so it must be path-only: a leading `//` is a protocol-relative
/// authority, and anything not starting with `/` could carry its own
/// scheme/host — either would let a caller steer the request off the HTTP
/// worker (SSRF). Whitespace and control characters are rejected for the same
/// reason.
fn validate(method: &str, path: &str) -> Result<String, String> {
    let method = method.to_ascii_uppercase();
    if !ALLOWED_METHODS.contains(&method.as_str()) {
        return Err(format!("method not allowed: {method}"));
    }
    if !path.starts_with('/') || path.starts_with("//") {
        return Err("path must start with a single '/'".to_string());
    }
    if path.chars().any(|c| c.is_control() || c == ' ') {
        return Err("path contains whitespace or control characters".to_string());
    }
    Ok(method)
}

/// Turn a configured bind host into a URL authority host. The console runs
/// beside the HTTP worker, so a wildcard/loopback bind (v4 or v6) is always
/// reachable at 127.0.0.1 — and that sidesteps the LAN hostname rewrite the
/// browser needed when it made the call itself. A real IPv6 literal must be
/// bracketed to be a valid URL authority.
fn normalize_host(host: &str) -> String {
    match host {
        "0.0.0.0" | "localhost" | "" | "::" | "::1" => "127.0.0.1".to_string(),
        other if other.contains(':') => format!("[{other}]"),
        other => other.to_string(),
    }
}

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

fn upstream_error(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

pub async fn probe_handler(
    State(state): State<AppState>,
    Json(req): Json<ProbeRequest>,
) -> Response {
    let Some(iii) = state.iii.clone() else {
        return upstream_error("probe is unavailable: no engine client");
    };

    let method = match validate(&req.method, &req.path) {
        Ok(method) => method,
        Err(message) => return bad_request(message),
    };
    for name in req.headers.keys() {
        if !ALLOWED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return bad_request(format!("header not allowed on this interface: {name}"));
        }
    }

    let base = match resolve_http_base(&iii, state.namespace.as_deref()).await {
        Ok(base) => base,
        Err(message) => return upstream_error(message),
    };
    let url = format!("{base}{}", req.path);

    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => return upstream_error(format!("http client init failed: {error}")),
    };

    let reqwest_method = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(method) => method,
        Err(_) => return bad_request(format!("invalid method: {method}")),
    };
    let mut builder = client.request(reqwest_method, &url);
    for (name, value) in &req.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = req.body {
        builder = builder.body(body);
    }

    let mut response = match builder.send().await {
        Ok(response) => response,
        Err(error) => return upstream_error(format!("request to {url} failed: {error}")),
    };

    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    // Read chunk by chunk and stop at the cap, so a large or fast upstream
    // cannot exhaust console memory. `from_utf8_lossy` decodes the bounded
    // buffer safely even when the cap lands inside a multibyte character.
    // End of body, or a mid-stream read error, ends the loop: a partial body
    // is an acceptable probe result — the status is what the panel needs.
    let mut bytes: Vec<u8> = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = MAX_BODY_BYTES - bytes.len();
        if chunk.len() >= room {
            bytes.extend_from_slice(&chunk[..room]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = String::from_utf8_lossy(&bytes).into_owned();

    Json(json!({
        "status": status,
        "location": location,
        "contentType": content_type,
        "body": body,
    }))
    .into_response()
}

/// Read the HTTP worker's `host`/`port` from its configuration entry, in the
/// console's own namespace, and build its base URL. Mirrors the panel's
/// former client-side resolution; `http` is the current worker id and
/// `iii-http` its deprecated predecessor.
async fn resolve_http_base(
    iii: &iii_sdk::IIIClient,
    namespace: Option<&str>,
) -> Result<String, String> {
    // The HTTP worker's config lives in the console's operating namespace;
    // fall back to "default" when the console runs without one.
    let namespace = namespace.unwrap_or("default");
    for id in ["http", "iii-http"] {
        let request = TriggerRequest {
            function_id: "configuration::get".to_string(),
            payload: json!({ "id": id }),
            action: None,
            timeout_ms: Some(5000),
        }
        .namespace(namespace);
        let Ok(entry) = iii.trigger(request).await else {
            continue;
        };
        let value = entry.get("value").unwrap_or(&Value::Null);
        let Some(port) = value.get("port").and_then(Value::as_u64) else {
            continue;
        };
        let host = value
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("127.0.0.1");
        return Ok(format!("http://{}:{port}", normalize_host(host)));
    }
    Err("no http worker configuration with a port found".to_string())
}

#[cfg(test)]
mod tests {
    use super::{normalize_host, validate};

    #[test]
    fn normalizes_hosts_for_url_authorities() {
        for wildcard in ["0.0.0.0", "localhost", "", "::", "::1"] {
            assert_eq!(normalize_host(wildcard), "127.0.0.1");
        }
        // A real IPv6 literal is bracketed so `http://<host>:<port>` parses.
        assert_eq!(normalize_host("fe80::1"), "[fe80::1]");
        // A plain v4 or hostname is passed through untouched.
        assert_eq!(normalize_host("10.0.0.4"), "10.0.0.4");
        assert_eq!(normalize_host("http.internal"), "http.internal");
    }

    #[test]
    fn accepts_catalog_methods_and_normal_paths() {
        assert_eq!(validate("get", "/s/home").unwrap(), "GET");
        assert_eq!(validate("POST", "/links?x=1").unwrap(), "POST");
        // A funny-looking but host-safe path stays on the http worker.
        assert!(validate("GET", "/@evil.com/x").is_ok());
    }

    #[test]
    fn rejects_off_worker_paths_and_bad_methods() {
        assert!(validate("GET", "//evil.com/").is_err()); // protocol-relative authority
        assert!(validate("GET", "https://evil.com").is_err()); // own scheme/host
        assert!(validate("GET", "s/home").is_err()); // no leading slash
        assert!(validate("GET", "/a b").is_err()); // whitespace
        assert!(validate("GET", "/a\nb").is_err()); // control char
        assert!(validate("TRACE", "/s/home").is_err()); // method not in the set
    }
}
