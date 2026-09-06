//! Upstream failures mapped to stable `provider/<code>` prefixes so callers
//! branch on the code, never the prose.
use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;

/// Map router bus errors surfaced through `router::provider::resolve`.
pub fn classify_bus_error(err: &Error) -> ErrorKind {
    match err {
        Error::Remote { code, .. } if code == "router/registration_rejected" => {
            ErrorKind::Permanent
        }
        _ => ErrorKind::Transient,
    }
}

/// An HTTP failure from the ElevenLabs API as a coded handler error. The
/// body excerpt carries ElevenLabs' `detail.message` when it has one.
pub fn upstream_status(status: u16, body: &str) -> Error {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/detail/message")
                .or_else(|| v.pointer("/detail"))
                .and_then(|d| d.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let code = match status {
        401 | 403 => "provider/auth_expired",
        402 => "provider/quota_exceeded",
        422 => "provider/invalid_input",
        429 => "provider/rate_limited",
        s if s >= 500 => "provider/upstream_transient",
        _ => "provider/upstream_status",
    };
    Error::Handler(format!("{code}: HTTP {status}: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_map_to_stable_prefixes() {
        let auth = upstream_status(
            401,
            r#"{"detail":{"status":"invalid_api_key","message":"bad key"}}"#,
        );
        assert!(auth.to_string().contains("provider/auth_expired"));
        assert!(auth.to_string().contains("bad key"));
        assert!(upstream_status(429, "slow down")
            .to_string()
            .contains("provider/rate_limited"));
        assert!(upstream_status(503, "")
            .to_string()
            .contains("provider/upstream_transient"));
    }
}
