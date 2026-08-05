//! Shared scaffolding for provider workers. Every provider worker carries
//! the same plumbing around its provider-specific wire code: the router
//! protocol client, registration-token persistence, the event pump into the
//! router-owned channel, SSE transport buffering, the tool-name codec, and
//! tiktoken-based local prompt token counting. These were verbatim copies
//! across the provider crates (the pump literally carried a "shared
//! extraction into llm-router is a listed follow-up" comment); this module is
//! that extraction. Providers keep only what is genuinely theirs: request
//! building, SSE event decoding, and error classification.

pub mod aborts;
pub mod cache;
pub mod chat_framing;
pub mod names;
pub mod pump;
pub mod router_client;
pub mod sse_transport;
pub mod state;
pub mod tiktoken_count;
pub mod vocabulary_count;
