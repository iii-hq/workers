//! provider-github-copilot: GitHub Copilot subscription provider behind
//! llm-router. Sign in with GitHub once (device flow) and the models the
//! subscription grants appear in the picker — the same one-login story other
//! mainstream harnesses ship.
//!
//! Wire is OpenAI Chat Completions against the Copilot API endpoint; what
//! makes this provider different is the credential lifecycle, which it owns
//! end to end (no api_key): a long-lived GitHub OAuth token (device-flow
//! login, env, or a read-only import of an existing editor credential) is
//! exchanged at `copilot_internal/v2/token` for a short-lived Copilot bearer
//! that also names the API endpoint; the bearer is cached and refreshed
//! proactively. Spec: tech-specs/2026-06-agentic/llm-router.md § The provider
//! protocol.

pub mod auth;
pub mod catalog;
pub mod config;
pub mod discovery;
pub mod errors;
pub mod exchange;
pub mod login;
pub mod manifest;
pub mod register;
pub mod request;
pub mod router_client;
pub mod sse;
pub mod state;
pub mod stream_fn;
pub mod surface;
pub mod upstream;
pub mod wire;

/// The provider id — also the `provider::<id>::*` function prefix and the
/// router config slice key.
pub const PROVIDER_ID: &str = "github-copilot";

/// Millisecond timestamps for AssistantMessage frames.
#[allow(dead_code)]
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
