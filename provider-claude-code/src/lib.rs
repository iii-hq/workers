//! provider-claude-code: Claude Code (Pro/Max subscription) Messages API
//! provider behind llm-router. Reuses the Claude Code CLI's OAuth credentials
//! (auth-credentials vault, or a local ~/.claude/.credentials.json dev
//! fallback) and calls the Anthropic Messages API with a Bearer token — the
//! subscription analog of provider-openai-codex. API keys belong on
//! provider-anthropic.
//! Spec: tech-specs/2026-06-agentic/llm-router.md § The provider protocol.

pub mod auth;
pub mod config;
pub mod curated;
pub mod discovery;
pub mod errors;
pub mod manifest;
pub mod register;
pub mod request;
pub mod router_client;
pub mod sse;
pub mod state;
pub mod stream_fn;
pub mod surface;
pub mod thinking;
pub mod upstream;
pub mod wire;

/// The provider id — also the `provider::<id>::*` function prefix and the
/// router config slice key.
pub const PROVIDER_ID: &str = "claude-code";

/// Millisecond timestamps for AssistantMessage frames.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
