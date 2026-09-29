//! cloudflared `--output json` is NDJSON logging, not a standalone URL object.
//! The URL is a whole boxed line in `message`; see README for upstream sources.
use serde::Deserialize;
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum LogEvent {
    Url(String),
    Connected,
    Disconnected,
}

#[derive(Deserialize)]
struct Record {
    message: String,
    #[serde(default)]
    level: String,
    #[serde(default, rename = "connIndex")]
    conn_index: Option<u8>,
    #[serde(default)]
    connection: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
}

pub fn public_url(input: &str) -> Option<String> {
    // Reject normalization tricks (userinfo, ports, escapes, backslashes, paths).
    let host = input.strip_prefix("https://")?;
    let label = host.strip_suffix(".trycloudflare.com")?;
    if label.is_empty()
        || label.len() > 63
        || !label.as_bytes().first()?.is_ascii_alphanumeric()
        || !label.as_bytes().last()?.is_ascii_alphanumeric()
        || !label
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return None;
    }
    let url = Url::parse(input).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| input.to_owned())
}

pub fn parse(line: &str) -> Option<LogEvent> {
    let record: Record = serde_json::from_str(line).ok()?;
    if record.level == "info" {
        let message = record.message.trim();
        let candidate = message
            .strip_prefix('|')
            .and_then(|s| s.strip_suffix('|'))
            .unwrap_or(message)
            .trim();
        if let Some(url) = public_url(candidate) {
            return Some(LogEvent::Url(url));
        }
    }
    if record.conn_index != Some(0) {
        return None;
    }
    if record.level == "info"
        && record.message == "Registered tunnel connection"
        && record
            .connection
            .as_deref()
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
        && matches!(record.protocol.as_deref(), Some("quic" | "http2"))
    {
        return Some(LogEvent::Connected);
    }
    if record.message.starts_with("Retrying connection in up to ")
        || record
            .message
            .starts_with("Restarting connection due to reconnect signal")
        || matches!(
            record.message.as_str(),
            "Serve tunnel error"
                | "failed to serve tunnel connection"
                | "Unregistered tunnel connection"
        )
    {
        return Some(LogEvent::Disconnected);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_lookalikes_credentials_paths_and_multiple_hosts() {
        for url in [
            "http://one.trycloudflare.com",
            "https://a.b.trycloudflare.com",
            "https://trycloudflare.com",
            "https://a.trycloudflare.com.evil.test",
            "https://u:p@a.trycloudflare.com",
            "https://a.trycloudflare.com:443",
            "https://a.trycloudflare.com/",
            "https://a.trycloudflare.com?q=1",
            "https://a.trycloudflare.com#x",
            "https://-a.trycloudflare.com",
            "https://a%2e.trycloudflare.com",
        ] {
            assert_eq!(public_url(url), None, "{url}");
        }
    }
    #[test]
    fn parses_json_box_not_unstructured_log_or_unrelated_text() {
        assert_eq!(
            parse(r#"{"level":"info","message":"|  https://quiet-marble.trycloudflare.com  |"}"#),
            Some(LogEvent::Url(
                "https://quiet-marble.trycloudflare.com".into()
            ))
        );
        assert_eq!(parse("INF https://quiet-marble.trycloudflare.com"), None);
        assert_eq!(
            parse(
                r#"{"level":"info","message":"request failed for https://quiet-marble.trycloudflare.com"}"#
            ),
            None
        );
    }
    #[test]
    fn connection_requires_structured_registration_fields() {
        assert_eq!(
            parse(r#"{"level":"info","message":"Registered tunnel connection"}"#),
            None
        );
        assert_eq!(
            parse(
                r#"{"level":"info","message":"Registered tunnel connection","connIndex":0,"connection":"72ff9dc2-84c6-4bb5-a536-349cf6dfe71d","protocol":"quic"}"#
            ),
            Some(LogEvent::Connected)
        );
    }
}
