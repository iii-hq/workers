//! Origin validation for the Streamable HTTP endpoint.
//!
//! The MCP spec requires a server to validate `Origin` so a web page cannot
//! drive a local MCP server through the browser (DNS rebinding included).
//! With `iii_call` behind it, that page could otherwise call any function.
//!
//! Non-browser clients (Claude Code, Codex, curl) send no `Origin` and pass.
//! A browser request passes when its origin is loopback (any scheme and port)
//! or is listed in `allowed_origins`; `"null"` and malformed origins never do.

use serde_json::Value;

/// The `Origin` header from an `HttpRequest.headers` object: any case, a
/// string or a list (first value). `None` when absent.
pub fn origin_header(headers: &Value) -> Option<String> {
    header(headers, "origin")
}

/// One header from an `HttpRequest.headers` object: any case, a string or a
/// list (first value). `None` when absent.
pub fn header(headers: &Value, name: &str) -> Option<String> {
    let obj = headers.as_object()?;
    let value = obj
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)?;
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.first().and_then(Value::as_str).map(str::to_string),
        _ => Some(String::new()),
    }
}

/// `(scheme, host)` of a serialized origin (`scheme://host[:port]`), or
/// `None` when it is not one.
fn parse_origin(origin: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = origin.split_once("://")?;
    if !matches!(scheme, "http" | "https") || rest.is_empty() || rest.contains('/') {
        return None;
    }
    let host = if let Some(stripped) = rest.strip_prefix('[') {
        let end = stripped.find(']')?;
        &rest[..end + 2]
    } else {
        rest.split(':').next()?
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme, host))
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "[::1]"
}

/// Whether a request with these headers may reach the bridge.
pub fn origin_allowed(headers: &Value, allowed_origins: &[String]) -> bool {
    let Some(origin) = origin_header(headers) else {
        return true;
    };
    if allowed_origins.iter().any(|a| a == &origin) {
        return true;
    }
    match parse_origin(&origin) {
        Some((_, host)) => is_loopback_host(host),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn none() -> Vec<String> {
        vec![]
    }

    #[test]
    fn no_origin_header_is_a_non_browser_client_and_passes() {
        assert!(origin_allowed(
            &json!({ "content-type": "application/json" }),
            &none()
        ));
        assert!(origin_allowed(&json!({}), &none()));
        assert!(origin_allowed(&serde_json::Value::Null, &none()));
    }

    #[test]
    fn loopback_origins_pass_on_any_port_and_scheme() {
        for o in [
            "http://localhost:3000",
            "http://127.0.0.1",
            "https://localhost",
            "http://[::1]:8080",
            "http://app.localhost:5173",
        ] {
            assert!(origin_allowed(&json!({ "origin": o }), &none()), "{o}");
        }
    }

    #[test]
    fn foreign_origins_are_refused() {
        for o in [
            "https://evil.example",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil.example",
            "http://192.168.1.10:3111",
        ] {
            assert!(!origin_allowed(&json!({ "origin": o }), &none()), "{o}");
        }
    }

    #[test]
    fn null_and_malformed_origins_are_refused() {
        for o in ["null", "", "localhost", "not a url", "file:///etc"] {
            assert!(!origin_allowed(&json!({ "origin": o }), &none()), "{o:?}");
        }
    }

    #[test]
    fn configured_origins_pass_exactly() {
        let allowed = vec!["https://console.example.com".to_string()];
        assert!(origin_allowed(
            &json!({ "origin": "https://console.example.com" }),
            &allowed
        ));
        assert!(!origin_allowed(
            &json!({ "origin": "https://console.example.com.evil" }),
            &allowed
        ));
        assert!(!origin_allowed(
            &json!({ "origin": "http://console.example.com" }),
            &allowed
        ));
    }

    #[test]
    fn the_header_name_is_case_insensitive_and_may_be_a_list() {
        assert!(!origin_allowed(
            &json!({ "Origin": "https://evil.example" }),
            &none()
        ));
        assert!(!origin_allowed(
            &json!({ "ORIGIN": ["https://evil.example"] }),
            &none()
        ));
        assert!(origin_allowed(
            &json!({ "Origin": ["http://localhost:1"] }),
            &none()
        ));
    }
}
