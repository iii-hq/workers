//! provider-openrouter: OpenRouter Chat Completions provider behind llm-router.
//! OpenRouter is an OpenAI Chat Completions-compatible aggregator fronting
//! hundreds of models from every major vendor behind one API key; this worker
//! forks the shared provider scaffolding, prefixes catalog ids with
//! `openrouter/`, and builds the catalog from OpenRouter's self-describing
//! `GET /api/v1/models` listing (context, pricing, capabilities — no local
//! metadata table to maintain).
//! Spec: tech-specs/2026-06-agentic/llm-router.md § The provider protocol.

pub mod catalog;
pub mod config;
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
pub mod upstream;
pub mod wire;

/// The provider id — also the `provider::<id>::*` function prefix and the
/// router config slice key.
pub const PROVIDER_ID: &str = "openrouter";

/// Millisecond timestamps for AssistantMessage frames.
#[allow(dead_code)]
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
