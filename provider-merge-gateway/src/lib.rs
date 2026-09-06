//! provider-merge-gateway: Merge Gateway (https://docs.merge.dev/merge-gateway) provider
//! behind llm-router, speaking the OpenAI Chat Completions-compatible surface
//! Merge Gateway exposes at https://api-gateway.merge.dev/v1/openai — one API
//! key routed by Merge across OpenAI, Anthropic, Google, and AWS Bedrock.
//! Spec: tech-specs/2026-06-agentic/llm-router.md § The provider protocol.

pub mod config;
pub mod count_tokens;
pub mod curated;
pub mod discovery;
pub mod embed;
pub mod errors;
pub mod manifest;
pub mod reasoning;
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
pub const PROVIDER_ID: &str = "merge-gateway";

/// Millisecond timestamps for AssistantMessage frames.
#[allow(dead_code)]
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
