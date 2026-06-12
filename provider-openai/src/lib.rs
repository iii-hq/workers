//! provider-openai: OpenAI Chat Completions provider behind llm-router.
//! Spec: tech-specs/2026-06-agentic/llm-router.md § The provider protocol.

pub mod config;
pub mod curated;
pub mod discovery;
pub mod errors;
pub mod manifest;
pub mod reasoning;
pub mod register;
pub mod request;
pub mod router_client;
pub mod sse;
pub mod state;
pub mod stream_fn;
pub mod upstream;
pub mod wire;

/// The provider id — also the `provider::<id>::*` function prefix and the
/// router config slice key.
pub const PROVIDER_ID: &str = "openai";

/// Millisecond timestamps for AssistantMessage frames.
#[allow(dead_code)]
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
