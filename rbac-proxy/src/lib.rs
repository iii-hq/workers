//! `rbac-proxy` worker library surface — exposed for integration tests and
//! so the binary (`main.rs`) and the test harness share one implementation.
//!
//! `rbac-proxy` is the [`console`](../../console) reverse-proxy with an RBAC
//! interceptor spliced into the frame pump: it opens its own public WebSocket
//! port, speaks the iii worker protocol verbatim, and transparently proxies
//! every frame (functions *and* channels) to a trusted engine listener —
//! authenticating the connection, gating each invocation, namespacing
//! registrations, gating trigger bindings, running middleware and registration
//! hooks, and rewriting the results of the built-in `engine::*` discovery
//! functions so a caller only ever sees what its boundaries allow.

pub mod channels;
pub mod config;
pub mod configuration;
pub mod engine_overrides;
pub mod functions;
pub mod interceptor;
pub mod manifest;
pub mod proxy;
pub mod rbac;
pub mod server;

pub fn worker_name() -> &'static str {
    "rbac-proxy"
}

/// Strip userinfo (`user:pass@`) from a URL before logging or returning it.
/// The engine WebSocket URL is operator-controlled and can carry credentials
/// in `wss://user:secret@host` form; the redactor keeps them out of `tracing`
/// output and out of the agent-callable `rbac-proxy::status` probe. Falls back
/// to the original string on parse failure.
pub fn redact_url(s: &str) -> String {
    match url::Url::parse(s) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        Err(_) => s.to_string(),
    }
}

#[cfg(test)]
mod redact_tests {
    use super::redact_url;

    #[test]
    fn redact_url_strips_userinfo_only() {
        assert_eq!(redact_url("ws://127.0.0.1:49134"), "ws://127.0.0.1:49134/");
        assert_eq!(
            redact_url("wss://user:secret@iii.example.com:1234/path"),
            "wss://iii.example.com:1234/path"
        );
        assert_eq!(redact_url("not a url"), "not a url");
    }
}
