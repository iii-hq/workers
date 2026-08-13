//! Safe provider-facing error messages. Classification may inspect the raw
//! upstream body locally, but the public assistant frame never carries it.
use crate::types::events::ErrorKind;

pub fn public_http_error(provider: &str, status: u16, kind: ErrorKind) -> String {
    match kind {
        ErrorKind::AuthExpired => {
            format!("{provider} authentication failed (HTTP {status}); refresh credentials")
        }
        ErrorKind::RateLimited => {
            format!("{provider} rate limit reached (HTTP {status}); retry later")
        }
        ErrorKind::ContextOverflow => {
            format!("request exceeds the {provider} context limit (HTTP {status})")
        }
        ErrorKind::Transient => {
            format!("{provider} is temporarily unavailable (HTTP {status})")
        }
        ErrorKind::Permanent => format!("{provider} rejected the request (HTTP {status})"),
    }
}

pub fn public_transport_error(provider: &str) -> String {
    format!("{provider} request failed before a response; inspect provider logs")
}

pub fn public_error(provider: &str, kind: ErrorKind) -> String {
    match kind {
        ErrorKind::AuthExpired => {
            format!("{provider} authentication is unavailable; refresh credentials")
        }
        ErrorKind::RateLimited => format!("{provider} rate limit reached; retry later"),
        ErrorKind::ContextOverflow => format!("request exceeds the {provider} context limit"),
        ErrorKind::Transient => format!("{provider} is temporarily unavailable"),
        ErrorKind::Permanent => format!("{provider} rejected the request"),
    }
}

pub fn public_protocol_error(provider: &str) -> String {
    format!("{provider} returned an invalid response; inspect provider logs")
}

pub fn public_catalog_error(provider: &str) -> String {
    format!("{provider} model discovery failed; inspect provider logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_messages_never_echo_an_upstream_body() {
        let message = public_http_error("openai", 401, ErrorKind::AuthExpired);
        assert_eq!(
            message,
            "openai authentication failed (HTTP 401); refresh credentials"
        );
        assert!(!message.contains("api_key"));
    }
}
